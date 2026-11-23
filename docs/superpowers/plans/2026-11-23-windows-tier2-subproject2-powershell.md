# Sub-project 2 Implementation Plan — PowerShell Shell Integration (`OSC 133` / `OSC 7`) & Task Runner

**Goal:** Enable full PowerShell shell integration (`OSC 133` prompt/command lifecycle markers and `OSC 7` live CWD tracking), Windows shell profile discovery, fast non-blocking `foreground_is_shell` + `local_shell_cwd` on ConPTY, and PowerShell-aware task quoting & execution on Windows.

**Architecture & Constraints:**
- PowerShell (`pwsh.exe` and Windows PowerShell 5.1 `powershell.exe`) is bootstrapped via `-NoLogo -NoExit -ExecutionPolicy Bypass -File <config_dir>/shell-integration/powershell/shellIntegration.ps1`.
- The PowerShell script uses `[char]0x1b` / `[char]7` for PS 5.1 compatibility, replays `$PROFILE` unless `CROFT_PS_NO_PROFILE=1`, restores `CROFT_PANE_BIN` at the front of `PATH`, binds `Ctrl+e` (`EndOfLine`) and `Ctrl+u` (`BackwardDeleteLine`) in `PSReadLine` so `CLEAR_TYPED_LINE` (`\x05x\x15`) clears typed input cleanly in Windows edit mode, emits `OSC 133;D;<exit>`, `OSC 7;file://<host>/<path>`, `OSC 133;A`, `<prompt>`, and `OSC 133;B` around prompts, and emits `OSC 133;C` on accepted commands.
- `OscSniffer` (`src/shell_integration.rs`) strips the leading `/` before Windows drive letters in `file://` OSC 7 URLs (`/C:/...` -> `C:/...`).
- `PtyTerminal` tracks an `AtomicU8` (`osc133_state`) updated by the reader thread on `OSC 133` `A`/`B`/`C`/`D` marks so `foreground_is_shell()` answers immediately without sleeping when `process_group_leader()` is `None`, and `local_shell_cwd()` trusts valid local `OSC 7` directory paths when `!cwd_is_observable()`.
- `src/tasks.rs` introduces `ShellFlavor::{Posix, PowerShell, Cmd}`, `Quoting::apply_for`, and `Task::command_line_for` so existing POSIX tests keep passing unchanged while Windows tasks receive valid PowerShell quoting and `& { Push-Location ...; $env:K = ...; try { ... } finally { Pop-Location } }` wrappers.

---

## Step 1: PowerShell Integration Script & OSC 7 Drive-Letter Parsing (`src/shell_integration.rs`)
1. Update `OscKind::Cwd` in `parse_event` to strip leading `/` before `<ascii_alpha>:` after percent-decoding.
2. Add `pub const POWERSHELL_INTEGRATION: &str` and `pub fn ensure_powershell_shim(config_dir: &std::path::Path) -> std::io::Result<PathBuf>`.
3. Add unit tests `osc7_strips_leading_slash_before_windows_drive_letter` and `powershell_shim_written_idempotently_and_contains_osc_markers`.

## Step 2: Shell Env, Profile Discovery & Re-exports (`src/widgets/terminal/shell_env.rs`, `src/widgets/terminal.rs`, `src/app/terminal_panes.rs`)
1. Add `"WT_SESSION"` and `"WT_PROFILE_ID"` to `HOST_TERMINAL_VARS`.
2. Add `shell_base_normalized(shell_path: &str) -> String`, update `interactive_shell_invocation` for `pwsh`/`powershell` (`["-NoLogo"]`), and update `apply_shell_integration_env` for `pwsh`/`powershell` (`-NoLogo -NoExit -ExecutionPolicy Bypass -File <shim>` + `CROFT_PS_NO_PROFILE=1`).
3. Add `pub fn discover_windows_shells() -> Vec<(String, String)>` probing `pwsh.exe`, `powershell.exe`, `cmd.exe`, Git Bash, and `wsl.exe`.
4. Re-export `discover_windows_shells` and `interactive_shell_invocation` from `src/widgets/terminal.rs`.
5. Update `App::terminal_profiles` (`src/app/terminal_panes.rs`) to fall back to `discover_windows_shells()` and `default_shell()` on Windows when `/etc/shells` is empty.

## Step 3: OSC 133 Prompt/Running State & OSC 7 CWD Tracking (`src/widgets/terminal.rs`, `src/widgets/terminal/spawn.rs`, `src/widgets/terminal/processes.rs`, `src/app/runners.rs`)
1. Add `OSC133_UNKNOWN`/`OSC133_PROMPT`/`OSC133_RUNNING` and `osc133_state: Arc<std::sync::atomic::AtomicU8>` on `PtyTerminal`.
2. Update `PtyTerminal::spawn_from` (`spawn.rs`) reader thread to update `osc133_for_thread` on `PromptStart`, `PromptEnd`, `CommandStart`, `CommandEnd`.
3. Update `local_shell_cwd()` (`processes.rs`) to return `claim.is_dir().then_some(claim)` when `!cwd_is_observable()` and the reporting host is local.
4. Update `foreground_is_shell()` (`processes.rs`) when `process_group_leader()` is `None` to check reader-thread finish / `is_pid_alive`, consult `osc133_state` (`OSC133_PROMPT` -> `true`, `OSC133_RUNNING` -> `false`), and otherwise keep the existing fallback.
5. Update `pane_is_idle_at` (`src/app/runners.rs`) to use `local_shell_cwd()` when `!cwd_is_observable()`.

## Step 4: PowerShell-Aware Task Quoting & Execution (`src/tasks.rs`, `src/app/runners.rs`)
1. Add `pub enum ShellFlavor { Posix, PowerShell, Cmd }` with `from_shell_path` and `host_default`.
2. Add `Quoting::apply_for`, `expand_task_text_for`, and `Task::command_line_for` (keeping `Quoting::apply` and `Task::command_line` defaulting to `ShellFlavor::Posix`).
3. Check both `node_modules/.bin/tsc` and `node_modules/.bin/tsc.cmd` in `tsconfig_tasks`.
4. Update `App::task_command_line_in` (`src/app/runners.rs`) to call `task.command_line_for(..., crate::tasks::ShellFlavor::host_default())`.

## Step 5: Unit & Integration Tests (`src/shell_integration.rs`, `src/tasks.rs`, `tests/platform.rs`)
- Verify via `cargo test --test platform`, `cargo test --bin croft shell_integration`, `cargo test --bin croft tasks::tests`, and `cargo test --bin croft parse_shells`.
