use std::io;
use std::process::{Child, Command};

/// Configure `cmd` so it detaches from croft's controlling terminal / console
/// window and cannot hijack raw-mode input or pop up a console window.
pub fn detach_from_tty(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// Cloneable handle to terminate a spawned process tree (Unix process group;
/// Windows Job Object + process PID).
///
/// On Windows, holding a `TreeKiller` keeps the underlying
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object handle open; dropping the
/// last clone closes the Job Object and cleans up any surviving descendants.
#[derive(Clone)]
pub struct TreeKiller {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: Option<std::sync::Arc<WinHandle>>,
    #[cfg(windows)]
    pid: u32,
}

impl TreeKiller {
    /// Forcefully terminate every process in the target's tree.
    pub fn kill_tree(&self) {
        #[cfg(unix)]
        if self.pgid > 0 {
            unsafe {
                libc::kill(-self.pgid, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        terminate_job(self.job.as_deref(), self.pid);
    }
}

/// A spawned child whose entire descendant process tree can be terminated
/// cleanly (Unix process group; Windows anonymous Job Object with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
pub struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    job: Option<std::sync::Arc<WinHandle>>,
}

#[cfg(windows)]
struct WinHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for WinHandle {}

// SAFETY: `WinHandle` wraps a Win32 Job Object handle passed only to thread-safe
// Win32 kernel APIs (`TerminateJobObject`, `SetInformationJobObject`, `CloseHandle`).
#[cfg(windows)]
unsafe impl Sync for WinHandle {}

#[cfg(windows)]
impl Drop for WinHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn attach_job_object(child: &Child) -> Option<std::sync::Arc<WinHandle>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_ok = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let assign_ok = if set_ok != 0 {
            AssignProcessToJobObject(h, child.as_raw_handle())
        } else {
            0
        };
        if assign_ok != 0 {
            Some(std::sync::Arc::new(WinHandle(h)))
        } else {
            CloseHandle(h);
            None
        }
    }
}

#[cfg(windows)]
fn terminate_job(job: Option<&WinHandle>, pid: u32) {
    if let Some(job) = job {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0, 1);
        }
    }
    force_kill_pid(pid);
}

/// Build a [`TreeKiller`] for an already-spawned `Child` that leads its own
/// session or process group (for example, configured via [`detach_from_tty`]).
pub fn tree_killer_for_leader(child: &Child) -> TreeKiller {
    #[cfg(unix)]
    {
        TreeKiller {
            pgid: i32::try_from(child.id()).unwrap_or(0),
        }
    }
    #[cfg(windows)]
    {
        TreeKiller {
            job: attach_job_object(child),
            pid: child.id(),
        }
    }
}

impl ManagedChild {
    pub fn spawn_in_group(cmd: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
            let child = cmd.spawn()?;
            Ok(Self { child })
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
            };

            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            let child = cmd.spawn()?;
            let job = attach_job_object(&child);
            Ok(Self { child, job })
        }
    }

    #[allow(dead_code)]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Transfer ownership of the underlying `Child` together with a cloneable
    /// [`TreeKiller`] handle.
    ///
    /// Unlike [`Self::into_child`], on Windows the `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
    /// Job Object remains active as long as any clone of the returned `TreeKiller`
    /// stays alive.
    pub fn split_killer(self) -> (Child, TreeKiller) {
        #[cfg(unix)]
        let killer = TreeKiller {
            pgid: i32::try_from(self.child.id()).unwrap_or(0),
        };
        #[cfg(windows)]
        let killer = TreeKiller {
            job: self.job,
            pid: self.child.id(),
        };
        (self.child, killer)
    }

    /// Transfer ownership of the underlying `Child` out of `ManagedChild`,
    /// leaving the child running (on Windows, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
    /// is cleared before the job handle is dropped).
    #[allow(dead_code)]
    pub fn into_child(self) -> Child {
        #[cfg(windows)]
        if let Some(ref job) = self.job {
            use windows_sys::Win32::System::JobObjects::{
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };
            unsafe {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = 0;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
        }
        self.child
    }

    pub fn kill_tree(&mut self) {
        #[cfg(unix)]
        {
            if let Ok(pid) = libc::pid_t::try_from(self.child.id())
                && pid > 0
            {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        {
            terminate_job(self.job.as_deref(), self.child.id());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        (unsafe { libc::kill(pid, 0) == 0 })
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
}

#[cfg(unix)]
fn send_signal_pid(pid: u32, sig: libc::c_int) -> bool {
    if pid == 0 {
        return false;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    pid > 0 && unsafe { libc::kill(pid, sig) == 0 }
}

#[cfg(windows)]
fn open_and_terminate(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 1);
        CloseHandle(h);
        ok != 0
    }
}

#[allow(dead_code)]
pub fn terminate_pid(pid: u32) -> bool {
    #[cfg(unix)]
    {
        send_signal_pid(pid, libc::SIGTERM)
    }
    #[cfg(windows)]
    {
        open_and_terminate(pid)
    }
}

pub fn force_kill_pid(pid: u32) -> bool {
    #[cfg(unix)]
    {
        send_signal_pid(pid, libc::SIGKILL)
    }
    #[cfg(windows)]
    {
        open_and_terminate(pid)
    }
}

pub fn raise_fd_limit() -> u64 {
    #[cfg(unix)]
    {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
            return 0;
        }
        let ceiling: libc::rlim_t = 1 << 20;
        let target = if lim.rlim_max == libc::RLIM_INFINITY {
            ceiling
        } else {
            lim.rlim_max.min(ceiling)
        };
        if lim.rlim_cur >= target {
            return lim.rlim_cur as u64;
        }
        let raised = libc::rlimit {
            rlim_cur: target,
            rlim_max: lim.rlim_max,
        };
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
            target as u64
        } else {
            lim.rlim_cur as u64
        }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

pub fn reset_sigpipe_default() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}
