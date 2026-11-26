use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// User home directory: `HOME` on Unix; `HOME` -> `USERPROFILE` ->
/// `HOMEDRIVE` + `HOMEPATH` on Windows.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(h));
    }
    #[cfg(windows)]
    {
        if let Some(up) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(up));
        }
        if let (Some(drive), Some(path)) = (
            std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty()),
            std::env::var_os("HOMEPATH").filter(|v| !v.is_empty()),
        ) {
            let mut full = drive;
            full.push(path);
            return Some(PathBuf::from(full));
        }
    }
    None
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        config_dir_windows(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("APPDATA"),
            home_dir(),
        )
    }
    #[cfg(not(windows))]
    {
        config_dir_unix(std::env::var_os("XDG_CONFIG_HOME"), home_dir())
    }
}

pub fn state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        state_dir_windows(
            std::env::var_os("XDG_STATE_HOME"),
            std::env::var_os("LOCALAPPDATA"),
            home_dir(),
        )
    }
    #[cfg(not(windows))]
    {
        state_dir_unix(std::env::var_os("XDG_STATE_HOME"), home_dir())
    }
}

pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        cache_dir_windows(std::env::var_os("LOCALAPPDATA"), home_dir())
    }
    #[cfg(not(windows))]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
            .join("croft")
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
pub fn config_dir_unix(xdg_config_home: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("croft")
}

#[cfg_attr(not(unix), allow(dead_code))]
pub fn state_dir_unix(xdg_state_home: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(xdg) = xdg_state_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return xdg.join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("croft")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn config_dir_windows(
    xdg_config_home: Option<OsString>,
    appdata: Option<OsString>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("croft");
    }
    if let Some(roaming) = appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(roaming).join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("croft")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn state_dir_windows(
    xdg_state_home: Option<OsString>,
    local_appdata: Option<OsString>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(xdg) = xdg_state_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return xdg.join("croft");
    }
    if let Some(local) = local_appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(local).join("croft").join("state");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("croft")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn cache_dir_windows(local_appdata: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(local) = local_appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(local).join("croft").join("cache");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("croft")
}

pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        if ok {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = String::from_utf8_lossy(&buf[..end]).into_owned();
            if !s.is_empty() {
                return s;
            }
        }
        String::from("local")
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{
            ComputerNameDnsHostname, GetComputerNameExW,
        };
        let mut len: u32 = 0;
        unsafe {
            GetComputerNameExW(ComputerNameDnsHostname, std::ptr::null_mut(), &mut len);
        }
        if len > 0 {
            let mut buf = vec![0u16; len as usize];
            let ok =
                unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) };
            if ok != 0
                && let Some(slice) = buf.get(..len as usize)
            {
                let s = String::from_utf16_lossy(slice);
                if !s.is_empty() {
                    return s;
                }
            }
        }
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("local"))
    }
}

pub fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("/bin/bash"))
    }
    #[cfg(windows)]
    {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if let Some(found) = find_on_path(candidate) {
                return found.to_string_lossy().into_owned();
            }
        }
        std::env::var("COMSPEC")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("cmd.exe"))
    }
}

#[cfg(windows)]
fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// True when `path` is a regular file that is executable (checks `0o111` on
/// Unix; checks `PATHEXT` extension on Windows).
pub fn is_executable_file(path: &Path) -> bool {
    super::fs::is_executable(path)
}

/// Find an executable named `cmd` inside `dir`.
///
/// On Unix, checks `dir.join(cmd)`.
/// On Windows, probes `.exe`, `.cmd`, `.bat` extensions via [`is_executable_file`]
/// (when `cmd` already carries an extension, checks that exact path directly)
/// so bare extensionless POSIX shims are ignored.
pub fn executable_in_dir(dir: &Path, cmd: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let candidate = dir.join(cmd);
        is_executable_file(&candidate).then_some(candidate)
    }
    #[cfg(windows)]
    {
        if Path::new(cmd).extension().is_some() {
            let candidate = dir.join(cmd);
            return is_executable_file(&candidate).then_some(candidate);
        }
        for ext in [".exe", ".cmd", ".bat"] {
            let candidate = dir.join(format!("{cmd}{ext}"));
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

/// Find the first executable matching `cmd` across `dirs`.
pub fn find_executable_in(cmd: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| executable_in_dir(dir, cmd))
}

/// Resolve `cmd` against the process `PATH`, returning the full path to the
/// matched executable (including `.exe`/`.cmd`/`.bat` on Windows).
pub fn resolve_on_path(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| executable_in_dir(&dir, cmd))
}

/// Canonical virtual-environment Python interpreter location inside `venv_dir`
/// (`Scripts\python.exe` on Windows, `bin/python` on Unix), whether or not
/// the venv exists yet.
pub fn venv_python_path(venv_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        venv_dir.join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        venv_dir.join("bin").join("python")
    }
}

/// Find an existing Python interpreter inside `venv_dir`, preferring native
/// layout for the host OS while still accepting POSIX/Windows venv layouts.
pub fn venv_python_in(venv_dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let candidates = [
        venv_dir.join("Scripts").join("python.exe"),
        venv_dir.join("bin").join("python"),
        venv_dir.join("Scripts").join("python3.exe"),
        venv_dir.join("bin").join("python3"),
    ];
    #[cfg(not(windows))]
    let candidates = [
        venv_dir.join("bin").join("python"),
        venv_dir.join("Scripts").join("python.exe"),
        venv_dir.join("bin").join("python3"),
        venv_dir.join("Scripts").join("python3.exe"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Locate the Go Delve debugger binary (`dlv` / `dlv.exe`) in Delve's own
/// resolution order: `PATH`, `GOBIN`, `GOPATH/bin` (default `~/go/bin`), then
/// croft's `~/.croft/servers/go` directory.
pub fn find_dlv_in(
    path_env: Option<&std::ffi::OsStr>,
    gobin: Option<&Path>,
    gopath: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    dirs.extend(gobin.map(Path::to_path_buf));
    match gopath {
        Some(gp) => dirs.push(gp.join("bin")),
        None => dirs.extend(home.map(|h| h.join("go").join("bin"))),
    }
    dirs.extend(home.map(|h| h.join(".croft").join("servers").join("go")));
    dirs.into_iter().find_map(|d| {
        #[cfg(unix)]
        {
            executable_in_dir(&d, "dlv").or_else(|| {
                let p = d.join("dlv");
                p.is_file().then_some(p)
            })
        }
        #[cfg(not(unix))]
        {
            executable_in_dir(&d, "dlv")
        }
    })
}

/// Rank an executable basename as an `lldb-dap` candidate, or `None` if it is
/// not one. Strips a trailing `.exe`/`.EXE` suffix so Windows LLVM binaries rank
/// identically to Unix ones: unversioned beats versioned, newer beats older,
/// and `lldb-dap` beats legacy `lldb-vscode` at equal version.
pub fn lldb_dap_rank(name: &str) -> Option<i64> {
    let base = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name);
    if base == "lldb-dap" {
        return Some(i64::MAX);
    }
    if base == "lldb-vscode" {
        return Some(i64::MAX - 1);
    }
    // `family` breaks ties at equal version: lldb-dap (1) over lldb-vscode (0).
    let (version, family) = base
        .strip_prefix("lldb-dap-")
        .map(|v| (v, 1))
        .or_else(|| base.strip_prefix("lldb-vscode-").map(|v| (v, 0)))?;
    let version: i64 = version.parse().ok()?;
    Some(version * 2 + family)
}

/// Scan `path_var` entries for the highest-ranked executable matching
/// `lldb-dap` / `lldb-dap-NN` / `lldb-vscode[-NN]`. Earlier `PATH` entries win
/// ties.
pub fn find_lldb_dap_on_path(path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let mut best: Option<(i64, PathBuf)> = None;
    for dir in std::env::split_paths(path_var) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(rank) = lldb_dap_rank(&name) else {
                continue;
            };
            let full = entry.path();
            if super::fs::is_executable(&full) && best.as_ref().is_none_or(|(r, _)| rank > *r) {
                best = Some((rank, full));
            }
        }
    }
    best.map(|(_, p)| p)
}
