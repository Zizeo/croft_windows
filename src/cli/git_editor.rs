//! `croft edit` as git's editor: the user's own `sequence.editor`, the editor git itself
//! would run, and running it as git does, for when no croft is there to edit in.

/// The user's own `sequence.editor` for the repository git is running in,
/// if they set one (#620).
pub(super) fn configured_sequence_editor() -> Option<String> {
    let out = crate::proc::command("git")
        .args(["config", "--get", "sequence.editor"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !value.is_empty()).then_some(value)
}

/// The editor git itself would run: `GIT_EDITOR`, `core.editor`, `VISUAL`,
/// `EDITOR`, then git's built-in default, as `git var GIT_EDITOR` reports.
/// `None` when git names none (a dumb terminal with no editor set): git
/// fails there, so no editor is invented. git's own reason reaches stderr.
pub(super) fn git_editor() -> Option<String> {
    crate::proc::command("git")
        .args(["var", "GIT_EDITOR"])
        .stderr(std::process::Stdio::inherit())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|editor| !editor.is_empty())
}

/// Locate Git for Windows's bundled POSIX `sh.exe` via `git --exec-path`,
/// falling back to `"sh"` if unavailable.
#[cfg(windows)]
fn find_git_sh() -> std::ffi::OsString {
    if let Ok(out) = std::process::Command::new("git")
        .arg("--exec-path")
        .stderr(std::process::Stdio::null())
        .output()
        && out.status.success()
    {
        let ep = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        for rel in [
            "../../../usr/bin/sh.exe",
            "../../../bin/sh.exe",
            "../../usr/bin/sh.exe",
            "../../bin/sh.exe",
        ] {
            let cand = ep.join(rel);
            if cand.is_file() {
                return cand.into_os_string();
            }
        }
    }
    std::ffi::OsString::from("sh")
}

/// Run the editor value `editor` on `path` exactly as git itself runs one,
/// and return its exit code.
pub(super) fn run_git_editor(editor: &str, path: &std::ffi::OsStr) -> i32 {
    #[cfg(unix)]
    let sh = std::ffi::OsString::from("sh");
    #[cfg(windows)]
    let sh = find_git_sh();
    let status = std::process::Command::new(sh)
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg(editor)
        .arg(path)
        .status();
    status.ok().and_then(|s| s.code()).unwrap_or(1)
}
