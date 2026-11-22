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

/// True when `path` is a regular file that is executable (checks `0o111` on Unix;
/// any regular file on Windows).
pub fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Find an executable named `cmd` inside `dir`.
///
/// On Unix, checks `dir.join(cmd)`.
/// On Windows, probes `.exe`, `.cmd`, `.bat` BEFORE the bare name `cmd` so that
/// in directories like `node_modules/.bin` or `%APPDATA%\npm` (which contain
/// both a bare POSIX `#!/bin/sh` script and a `<cmd>.cmd` batch wrapper),
/// the runnable Windows `.exe`/`.cmd` file always wins.
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
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
        for ext in [".exe", ".cmd", ".bat", ""] {
            let name = if ext.is_empty() {
                cmd.to_string()
            } else {
                format!("{cmd}{ext}")
            };
            let candidate = dir.join(name);
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
