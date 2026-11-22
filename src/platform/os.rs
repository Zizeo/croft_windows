use std::path::Path;
use std::process::{Command, Stdio};

/// Reveal `path` in the OS file manager (Finder on macOS, Explorer on Windows).
/// Returns `Ok(label)` describing the manager, or `Err(label)` on failure.
pub fn reveal_in_file_manager(path: &Path) -> Result<&'static str, &'static str> {
    #[cfg(target_os = "macos")]
    {
        if Command::new("open")
            .arg("-R")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            Ok("Revealed in Finder")
        } else {
            Err("Could not reveal in Finder")
        }
    }
    #[cfg(windows)]
    {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(dunce::simplified(path).as_os_str());
        if Command::new("explorer.exe")
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            Ok("Revealed in File Explorer")
        } else {
            Err("Could not reveal in File Explorer")
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Err("Reveal in file manager is not supported on this OS")
    }
}
