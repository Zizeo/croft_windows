use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

fn validate_open_target(target: &str) -> io::Result<()> {
    if target.is_empty()
        || target
            .chars()
            .any(|c| matches!(c, '\0' | '\r' | '\n' | '\t'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "open target must be non-empty and contain no control characters",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn shell_execute_open(target: &str) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
    use windows_sys::core::PCWSTR;
    const SW_SHOWNORMAL: i32 = 1;

    // `windows-sys` 0.59 gates `Win32::UI::Shell::ShellExecuteW` on
    // `Win32_UI_WindowsAndMessaging` (not enabled in Cargo.toml), so declare
    // the `shell32.dll` symbol directly with `windows-sys` ABI types.
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: HWND,
            lp_operation: PCWSTR,
            lp_file: PCWSTR,
            lp_parameters: PCWSTR,
            lp_directory: PCWSTR,
            n_show_cmd: i32,
        ) -> HINSTANCE;
    }

    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `verb` and `wide` are NUL-terminated UTF-16 buffers that outlive
    // the synchronous `ShellExecuteW` call; unused pointer arguments are null.
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = rc as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecuteW failed with code {code}"
        )))
    }
}

#[cfg(not(windows))]
fn unix_open_command(target: &str) -> Command {
    let program = if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        "open"
    };
    let mut cmd = Command::new(program);
    cmd.arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// Open `url` in the platform's default handler and wait for the launcher to
/// accept it (or report launch failure).
pub fn open_url(url: &str) -> io::Result<()> {
    validate_open_target(url)?;
    #[cfg(windows)]
    {
        shell_execute_open(url)
    }
    #[cfg(not(windows))]
    {
        let status = unix_open_command(url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("local open(1) exited {status}")))
        }
    }
}

/// Launch `target` (a URL or file path) with the platform's default handler
/// without blocking the caller on exit status.
pub fn spawn_open_target(target: &str) -> io::Result<()> {
    validate_open_target(target)?;
    #[cfg(windows)]
    {
        shell_execute_open(target)
    }
    #[cfg(not(windows))]
    {
        unix_open_command(target).spawn()?;
        Ok(())
    }
}

/// Whether revealing a path in the OS file manager is supported on this platform.
pub const fn reveal_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Platform-native menu label for revealing a file in the OS file manager.
pub const fn reveal_menu_label() -> &'static str {
    if cfg!(windows) {
        "Reveal in File Explorer"
    } else {
        "Reveal in Finder"
    }
}

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
