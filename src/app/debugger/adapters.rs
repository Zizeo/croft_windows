//! Finding and installing debug adapters: lldb-dap under each name LLVM ships it as, what
//! to say when it is missing, whether a file is a runnable binary, and Debug: Install Go
//! Debugger (delve).

use super::*;

/// Rank an executable basename as an lldb-dap candidate, or `None` if it is not
/// one. LLVM ships the adapter under several names: the bare `lldb-dap`, the
/// versioned `lldb-dap-18` that Debian/Ubuntu install (often with no
/// unversioned symlink), and the legacy `lldb-vscode[-NN]` name from older
/// releases. Higher rank wins: an unversioned binary (tracks the system
/// default) beats any versioned one, a newer LLVM release beats an older one,
/// and at equal version the modern `lldb-dap` name beats legacy `lldb-vscode`.
#[allow(dead_code)]
pub(in crate::app) fn lldb_dap_rank(name: &str) -> Option<i64> {
    crate::platform::env::lldb_dap_rank(name)
}

/// Resolve the `lldb-dap` adapter binary: the Xcode Command Line Tools copy
/// first (always present on a dev Mac), then the best-ranked match found by
/// scanning `PATH` for `lldb-dap`/`lldb-dap-NN`/`lldb-vscode[-NN]`, and on
/// Windows standard `Program Files\LLVM\bin\lldb-dap.exe` locations. `None` if
/// none is installed.
pub(in crate::app) fn lldb_dap_path() -> Option<String> {
    let xcode = "/Library/Developer/CommandLineTools/usr/bin/lldb-dap";
    if std::path::Path::new(xcode).is_file() {
        return Some(xcode.to_string());
    }
    if let Some(found) =
        crate::platform::env::find_lldb_dap_on_path(std::env::var_os("PATH").as_deref())
    {
        return Some(found.to_string_lossy().into_owned());
    }
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                dirs.push(std::path::PathBuf::from(base).join("LLVM").join("bin"));
            }
        }
        dirs.push(std::path::PathBuf::from(r"C:\Program Files\LLVM\bin"));
        for dir in dirs {
            if let Some(found) = crate::platform::env::executable_in_dir(&dir, "lldb-dap") {
                return Some(found.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// The error shown when no lldb-dap adapter is installed, with a host-correct
/// install hint: Xcode Command Line Tools on macOS, winget on Windows, or the
/// LLVM package elsewhere.
pub(in crate::app) fn lldb_dap_missing_message() -> String {
    let hint = if cfg!(target_os = "macos") {
        "install Xcode Command Line Tools"
    } else if cfg!(windows) {
        "install LLVM, e.g. winget install LLVM.LLVM"
    } else {
        "install LLVM, e.g. apt install lldb"
    };
    format!("lldb-dap not found ({hint})")
}

/// Whether `path` is a regular file with the executable bit set — i.e. a
/// compiled binary or script croft can run directly (`./file`) and debug via
/// lldb-dap without a build step. The honest signal for "directly executable",
/// since such files usually have no extension.
pub(in crate::app) fn is_executable_file(path: &Path) -> bool {
    crate::platform::fs::is_executable(path)
}

impl App {
    /// Debug: Install Go Debugger (delve) (#264). Runs `go install` in a
    /// terminal pane, where the user sees the command and its output: that
    /// visible run is the consent, as for a coverage tool. Without Go there
    /// is nothing to run it with, so it says that instead.
    pub(in crate::app) fn install_delve(&mut self) {
        if !crate::dap::install::go_on_path() {
            self.status = crate::dap::install::dlv_missing_message(false);
            return;
        }
        match crate::widgets::terminal::PtyTerminal::new(self.roots.primary()) {
            Ok(mut term) => {
                term.set_manual_name(Some(String::from("delve install")));
                let cmd = crate::dap::install::dlv_install_command(
                    crate::tasks::ShellFlavor::host_default(),
                );
                term.write_input(format!("{cmd}\r").as_bytes());
                self.insert_terminal(term);
                self.show_terminal = true;
                self.focus_pane(Pane::Terminal);
                self.status = String::from(
                    "Installing delve into ~/.croft/servers/go; start debugging again once it finishes",
                );
            }
            Err(e) => self.status = format!("Could not open a terminal: {e}"),
        }
    }
}
