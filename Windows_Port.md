# croft Windows Tier 2 Port — Remaining Sub-projects Plan (Sub-projects 2–6)

> **Repository:** `C:\Users\Elio\prog\croft_windows` (`github.com:Zizeo/croft_windows`, branch `windows-port`)  
> **Target Environment:** `x86_64-pc-windows-msvc`, Rust `1.97.1` (Edition `2024`), PowerShell 7+ (`pwsh.exe`), Windows PowerShell 5.1 (`powershell.exe`), `cmd.exe` fallback, hosted in Windows Terminal, WezTerm, or 3rd-party Windows terminal emulators.

---

## 1. Current Baseline (Completed)

Before Sub-projects 2–6, the following foundations are already implemented on branch `windows-port` (HEAD `a66ba1f`):

1. **Sub-project 1 — Compilable Core & Platform Layer:**
   - Relocated `libc = "0.2"` and `async-lsp`'s `"stdio"` feature to `[target.'cfg(unix)'.dependencies]`; added `dunce = "1"` and minimal `windows-sys = "0.59"`.
   - Replaced the `date` subprocess in `build.rs` with a pure-Rust UTC ISO-8601 formatter.
   - Built unified OS abstractions under `src/platform/` (`env`, `fs`, `process`, `time`, `clipboard`, `os`, `ipc`) with `unsafe` Win32 FFI confined strictly to `src/platform/`, plus `dunce::canonicalize` UNC stripping, Win32 Job Objects (`ManagedChild`, `TreeKiller`), and Win32 `CF_UNICODETEXT` clipboard support.
   - Added ConPTY channel-bridged PTY reading in `src/widgets/terminal/backend.rs` and `src/widgets/terminal/spawn.rs`.
   - Kept full `#[cfg(unix)]` compatibility and verified `cargo test --test platform` (`tests/platform.rs`).
2. **Windows LSP Discovery & Provisioning (`95018c0`):**
   - Added `.exe`/`.cmd`/`.bat` executable lookup (`src/platform/env.rs`, `src/lsp/manager/discovery.rs`), Windows `uv.exe` PowerShell bootstrap and Node/`npm.cmd` discovery (`src/lsp/install.rs`), and `windows-x86_64` binary targets/digests for `rust-analyzer`, `taplo`, `clangd`, `marksman`, and `csvlens`.
3. **Windows SSH Workspaces (`a66ba1f`):**
   - Ported `croft remote`, `croft sync-config`, and `SshAuth` (`src/remote/control.rs`, `src/remote_connect.rs`, `src/remote_bulk.rs`, `src/remote/*.rs`) to socket-free Windows OpenSSH (`ssh.exe`).

*(Tier-3 Unix-daemon / OpenSSH-`ControlMaster`-dependent features — `attach`, `ls`, `record`, `web`, `pane-host`, `session-host`, `collab`, `pair`, `devcontainer`, and `lsof`/`ss` socket inspection — remain permanently out of native Windows scope and direct users to WSL2.)*

---

## 2. Sub-project 2 — PowerShell Shell Integration (`OSC 133` / `OSC 7`) & Task Runner

### Problem Statement
- On Windows ConPTY, `PaneBackend::process_group_leader()` (`src/widgets/terminal/backend.rs`) and `cwd_of_pid` / `foreground_group_of` (`src/widgets/terminal/processes.rs:165-168, 357-361`) return `None` because Windows has no POSIX `tcgetpgrp(master)` or `/proc/<pid>/cwd`.
- `PtyTerminal::foreground_is_shell()` (`src/widgets/terminal/processes.rs:265-293`) falls back into its 5 ms sleep retry and returns `false` once `input_seen` is set, degrading idle-shell detection, pane CWD inheritance on splits, prompt-navigation marks (`OSC 133`), command duration tracking, and `CROFT_PANE_BIN` injection.
- `src/tasks.rs` (`Quoting::apply` at line 107, `Task::command_line` at lines 178–234) emits POSIX syntax (`'...'`, backslash escaping, and `(cd <dir> && export KEY=<val> && <cmd>)`), which breaks inside PowerShell (`pwsh.exe` / `powershell.exe`) and `cmd.exe` panes.

### Architecture & Changes
1. **PowerShell Shell Integration Script (`src/shell_integration.rs`)**
   - Add `POWERSHELL_INTEGRATION: &str` (`shellIntegration.ps1`) written to `<config_dir>/shell-integration/powershell/shellIntegration.ps1` via a new `pub fn ensure_powershell_shim(config_dir: &Path) -> std::io::Result<PathBuf>`.
   - Requirements for PowerShell 5.1 (`powershell.exe`) **and** PowerShell 7+ (`pwsh.exe`):
     - Avoid PowerShell 7-only escape literals (use `[char]0x1b` and `[char]7` instead of `` `e `` and `` `a ``).
     - Prepend `$env:CROFT_PANE_BIN` to `$env:PATH` if set so `croft` commands inside embedded panes resolve to the running instance.
     - Source `$PROFILE.CurrentUserAllHosts` and `$PROFILE.CurrentUserCurrentHost` if present (unless `$env:CROFT_PS_NO_PROFILE -eq '1'`), before wrapping the prompt so user themes/functions (Oh My Posh, Starship, custom `prompt`) remain intact.
     - Wrap `global:prompt`:
       - Emit `OSC 133;D;<exit>` after a command executes (reading `$?` and `$global:LASTEXITCODE`).
       - Emit `OSC 7;file://<hostname>/<drive>:/<forward-slash-path>` from `(Get-Location).ProviderPath`.
       - Emit `OSC 133;A`, call the preserved user prompt function, and emit `OSC 133;B`.
     - Hook `PSReadLine` `Enter` (`Set-PSReadLineKeyHandler` / `AcceptLine` wrapper, falling back cleanly if `PSReadLine` is unavailable) to emit `OSC 133;C` right before execution.
   - Update `OSC 7` URI-to-`PathBuf` decoding in `src/shell_integration.rs` so `file:///C:/Users/Elio` and `file://localhost/C:/Users/Elio` normalize `/C:/...` into `C:\Users\Elio`.

2. **Shell Launch & Profile Discovery (`src/widgets/terminal/shell_env.rs`)**
   - Update `interactive_shell_invocation` (`src/widgets/terminal/shell_env.rs:153`) and `apply_shell_integration_env` (`lines 20-99`):
     - Match Windows shell basenames case-insensitively (`pwsh`, `pwsh.exe`, `powershell`, `powershell.exe`, `cmd`, `cmd.exe`).
     - When `CROFT_SHELL_INTEGRATION != "0"` and `ensure_powershell_shim` succeeds, launch PowerShell with `["-NoLogo", "-NoExit", "-ExecutionPolicy", "Bypass", "-File", "<shim>"]`; otherwise `["-NoLogo"]`.
   - Add Windows shell profile discovery (`pwsh.exe`, `powershell.exe`, `cmd.exe`, Git Bash `C:\Program Files\Git\bin\bash.exe`, and `wsl.exe`) for the terminal profile picker.

3. **Prompt-State & OSC-7 CWD Fallback (`src/widgets/terminal/processes.rs`)**
   - When `cwd_is_observable()` (`src/widgets/terminal/processes.rs:178-180`) is `false` (Windows):
     - Teach `local_shell_cwd()` (`line 220`) to accept verified local-host `OSC 7` reports when `cwd_of_pid` is `None`.
     - Update `foreground_is_shell()` (`line 265`) so when `process_group_leader()` is `None`, it inspects the tracked `OSC 133` state (in prompt `A`/`B`/`D` vs running command `C`) before falling back, eliminating the 5 ms sleep stall.

4. **Shell-Aware Quoting in Task Runner (`src/tasks.rs`)**
   - Add `pub enum ShellFlavor { Posix, PowerShell, Cmd }` inferred from the target pane shell (`crate::platform::env::default_shell()`).
   - Add `Quoting::apply_for(self, value: &str, shell: ShellFlavor) -> String`:
     - `Strong`: `'...'` with `'` doubled as `''` for PowerShell.
     - `Weak`: `"..."` escaping `` ` ``, `"`, and `$` with backticks for PowerShell.
     - `Escape`: backtick-escaping PowerShell special characters.
   - Update `Task::command_line` (`lines 220-234`): for `ShellFlavor::PowerShell` when `cwd` or `env` is present, emit `& { Push-Location <cwd>; $env:KEY = <val>; try { <line> } finally { Pop-Location } }` instead of POSIX `(cd ... && export ... && ...)`.

### Verification
- Unit tests in `src/shell_integration.rs` and `src/tasks.rs` covering Windows `OSC 7` drive-letter URLs, PowerShell `Quoting::apply_for` edge cases (`'`, `$`, spaces), and `Task::command_line` PowerShell output.
- Smoke test running `pwsh.exe -NoLogo -NoExit -ExecutionPolicy Bypass -File shellIntegration.ps1` via ConPTY and verifying `OscSniffer` receives `PromptStart`, `PromptEnd`, `Cwd`, `CommandStart`, and `CommandEnd`.

---

## 3. Sub-project 3 — Host Terminal Capabilities (Windows Terminal & WezTerm)

### Problem Statement
- `StartupProbe` (`src/app/mod.rs:6135-6136, 6247-6256` and `src/app/host_integration.rs`) is gated to `#[cfg(unix)]` because it uses raw `STDIN_FILENO` + `libc::poll` to send and parse DA1 (`\x1b[c`) and Kitty keyboard (`\x1b[?u`) queries.
- Consequently on Windows:
  - `app.sixel_supported` and `SIXEL_HOST` stay `false`, disabling Sixel graphics in Windows Terminal 1.22+ and WezTerm.
  - `app.legacy_ctrl_punct` is never calibrated from a Kitty keyboard reply.
  - `crossterm::terminal::window_size()` reports pixel width/height as `0` on Windows (since Win32 console screen-buffer APIs only return cell counts), leaving `init_graphics` without accurate cell pixel dimensions for Sixel / OSC 1337 / SVG icon rasterization.

### Architecture & Changes
1. **Win32 Console Capability Probe (`src/platform/terminal.rs` — new module in `src/platform/mod.rs`)**
   - Define cross-platform `pub struct HostProbeResult { pub sixel: bool, pub kitty_keyboard: bool, pub answered: bool, pub cell_pixel_size: Option<(u16, u16)> }`.
   - Under `#[cfg(windows)]`, confine all `unsafe` Win32 console FFI inside `src/platform/terminal.rs`:
     - Save and restore `GetConsoleMode(STD_INPUT_HANDLE)`, temporarily enabling `ENABLE_VIRTUAL_TERMINAL_INPUT` (`0x0200`) and disabling `ENABLE_LINE_INPUT` / `ENABLE_ECHO_INPUT` (`ENABLE_PROCESSED_INPUT` off during raw query).
     - Write `\x1b[?u\x1b[16t\x1b[14t\x1b[c` to `STD_OUTPUT_HANDLE` (Kitty keyboard query, cell pixel size `CSI 16 t`, window pixel size `CSI 14 t`, and DA1 `CSI c`).
     - Wait on `WaitForSingleObject(stdin_handle, remaining_ms)` within a tight `150 ms` deadline (exiting immediately once the trailing `DA1` `c` terminator arrives, typically `< 10 ms`), reading VT response events via `ReadFile` / `ReadConsoleInputW` without leaving unread garbage or consuming unrelated user keystrokes.
     - Fallback cell pixel size: if the host emulator ignores `CSI 16 t` / `CSI 14 t`, query `GetCurrentConsoleFontEx` (`dwFontSize.X` / `dwFontSize.Y`) on `STD_OUTPUT_HANDLE`; if the emulator ignores VT queries altogether (legacy `conhost.exe`), time out cleanly with `sixel = false`, `kitty_keyboard = false`, `answered = false`.
   - Pure VT parser `parse_host_probe_bytes(buf: &[u8]) -> HostProbeResult` tested hermetically without a live console handle.

2. **App & Graphics Integration (`src/app/mod.rs`, `src/app/host_integration.rs`, `src/iterm2_inline.rs`)**
   - Wire `StartupProbe` in `src/app/mod.rs:6135-6136, 6247-6256` on `#[cfg(windows)]` using `crate::platform::terminal::probe_host_terminal`.
   - Update `cell_pixel_size` lookup in `App::init_graphics` (`src/iterm2_inline.rs` / `src/app/`) to fall back to `HostProbeResult::cell_pixel_size` (or window pixel ÷ `(cols, rows)`) whenever `crossterm::terminal::window_size()` returns `0` pixel dimensions.
   - Verify `src/iterm2_inline.rs` environment detection on Windows:
     - `TERM_PROGRAM=WezTerm` -> OSC 1337 / Sixel / Kitty keyboard supported cleanly.
     - `WT_SESSION` (Windows Terminal) -> Sixel via DA1 attribute `4` + `CSI 16 t` pixel dimensions.
     - Legacy `conhost.exe` -> clean vector-glyph / text UI fallback with zero visual artifacts.

### Verification
- Unit tests for `parse_host_probe_bytes` covering combined Kitty (`\x1b[?0u`), `CSI 16 t` (`\x1b[6;24;12t`), `CSI 14 t` (`\x1b[4;720;1440t`), and DA1 (`\x1b[?61;4;6c`) across split chunks and empty timeout buffers.
- Interactive verification in **Windows Terminal** (confirm crisp SVG codicons / inline Sixel images) and **WezTerm** (confirm Kitty keyboard chords + OSC 1337 / Sixel inline images).

---

## 4. Sub-project 4 — Windows Toolchains (DAP, Venvs, Task Binaries, & NTFS Save Hardening)

### Problem Statement
While Windows LSP binary discovery and provisioning (`src/lsp/manager/discovery.rs`, `src/lsp/install.rs`) landed in `95018c0`, several non-LSP toolchain and filesystem paths still assume POSIX conventions:
1. **DAP Python venv & tool resolution (`src/dap/install.rs`):**
   - `debug_venv_python()` (`src/dap/install.rs:48-50`) hardcodes `.join("bin").join("python")`, whereas Windows `uv venv` puts the interpreter at `debug-venv\Scripts\python.exe`.
   - `which(bin)` (`src/dap/install.rs:219-223`) checks `p.join(bin).is_file()` without `.exe`/`.cmd`/`.bat` extensions (breaking `which("node")`, `which("go")`, and `which("uv")`).
   - `find_dlv()` (`src/dap/install.rs:291-309`) checks `.join("dlv")` without `.exe`, and `DLV_INSTALL` (`line 321`) hardcodes `sh -c '...'`.
   - `ensure_debug_venv()` calls bare `"uv"` instead of reusing `crate::lsp::install::uv_program()` / bootstrapped `%USERPROFILE%\.croft\servers\uv-dist\uv.exe`.
   - Workspace Python venv detection (`src/dap/`) and `lldb-dap` lookup need Windows equivalents (`.venv\Scripts\python.exe`, `C:\Program Files\LLVM\bin\lldb-dap.exe`).
2. **TypeScript task check (`src/app/bottom_panel.rs`):**
   - `project_check_command()` (`src/app/bottom_panel.rs:500`) checks `root.join("node_modules").join(".bin").join("tsc").is_file()`, missing `tsc.cmd` on Windows.
3. **NTFS Save Contention (`src/platform/fs.rs`, `src/widgets/editor/disk.rs`, `src/prefs.rs`):**
   - On Windows NTFS, atomic saves via temp-file rename (`MoveFileExW`) or overwrite can transiently return `PermissionDenied` (`ERROR_ACCESS_DENIED` code `5` or `ERROR_SHARING_VIOLATION` code `32`) when Windows Defender, Search Indexer, or a running LSP (`rust-analyzer.exe`, `ty.exe`) briefly holds a read handle.

### Architecture & Changes
1. **Windows DAP Paths & Installers (`src/dap/install.rs`, `src/dap/`)**
   - Update `debug_venv_python()`:
     - `#[cfg(windows)]`: `debug_venv_dir()?.join("Scripts").join("python.exe")`
     - `#[cfg(unix)]`: `debug_venv_dir()?.join("bin").join("python")`
   - Replace `which(bin)` (`line 219`) with `crate::platform::env::resolve_on_path(bin).is_some()`.
   - Update `find_dlv` (`line 307`) to probe via `crate::platform::env::executable_in_dir(&dir, "dlv")` so `dlv.exe` resolves cleanly, and select a PowerShell-compatible `DLV_INSTALL` command on Windows (`powershell -Command ...`) alongside the Unix `sh -c` string.
   - Expose and reuse `crate::lsp::install::ensure_uv()` from `ensure_debug_venv()` when `uv` is not already on `PATH`.
   - Update workspace `.venv` discovery in `src/dap/` to probe both `<dir>\Scripts\python.exe` and `<dir>/bin/python`, and check `C:\Program Files\LLVM\bin\lldb-dap.exe` + `resolve_on_path("lldb-dap")` for LLDB debugging.

2. **Workspace `node_modules/.bin` Resolution (`src/app/bottom_panel.rs`)**
   - Replace `root.join("node_modules").join(".bin").join("tsc")` (`line 500`) with `crate::platform::env::executable_in_dir(&root.join("node_modules").join(".bin"), "tsc")` so `tsc.cmd` is found directly.

3. **NTFS Sharing-Violation Save Retry (`src/platform/fs.rs`, `src/widgets/editor/disk.rs`, `src/prefs.rs`)**
   - Add `pub fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()>` and `pub fn write_retrying(path: &Path, bytes: &[u8]) -> std::io::Result<()>` in `src/platform/fs.rs`:
     - Under `#[cfg(windows)]`, retry when `err.kind() == ErrorKind::PermissionDenied` or `err.raw_os_error()` is `5` (`ERROR_ACCESS_DENIED`) or `32` (`ERROR_SHARING_VIOLATION`) across up to 6 attempts with bounded backoff (`5, 10, 20, 40, 80, 150 ms`), while still refusing real read-only protection immediately via `write_protection(path)` (`src/widgets/editor/disk.rs:96-116`).
   - Wire save paths in `src/widgets/editor/disk.rs` and `src/prefs.rs` through `platform::fs::rename_retrying` / `write_retrying`.

### Verification
- Tests in `tests/platform.rs` for `rename_retrying`, `debug_venv_python` Windows `Scripts\python.exe` path, `find_dlv` resolving `dlv.exe`, and `tsc.cmd` lookup inside `node_modules\.bin`.

---

## 5. Sub-project 5 — Local Windows IPC (`croft view`, `croft edit`, `croft hook`) & `croft://` Links

### Problem Statement
- `src/platform/ipc.rs` currently provides inert stub structs `LocalListener` and `LocalStream` on Windows.
- `prepare_view_listener` (`src/app/host_integration.rs:1598-1607`), `src/view_ipc.rs`, `src/agent_approval.rs`, `src/agent_hook.rs`, and `src/cli/links.rs` compile on Windows but return `"Windows IPC not yet implemented"` / unsupported errors, disabling `croft view`, `croft edit --wait`, Claude Code pre-tool approval hooks (`croft hook` / `croft decide`), and `croft://` deep links.

### Architecture & Changes
1. **Cross-Platform `LocalListener` / `LocalStream` (`src/platform/ipc.rs`)**
   - Implement `#[cfg(windows)]` `LocalListener` and `LocalStream` using **loopback TCP (`127.0.0.1:0`) paired with a capability token file at `path`**:
     - *Why capability-authenticated loopback TCP instead of raw Win32 Named Pipes:*
       1. Exposes identical synchronous + non-blocking semantics (`set_nonblocking(true)`, `ErrorKind::WouldBlock`, `set_read_timeout`, `try_clone`, `Read`, `Write`) via Rust stdlib `std::net::{TcpListener, TcpStream}` with **zero `unsafe` Win32 overlapped-I/O code**.
       2. Avoids `AF_UNIX` 108-byte `SUN_LEN` path truncation under deep `%LOCALAPPDATA%\croft\cache` paths and avoids stale Named Pipe server-instance limits when polling from `App`'s single-threaded frame loop (`prepare_view_listener` and `accept_into`).
     - Protocol:
       - `LocalListener::bind(path: &Path) -> io::Result<Self>`: binds `TcpListener::bind("127.0.0.1:0")`, generates a random 16-byte hex secret (`nonce`), atomically writes `<port>\n<nonce>` to `path`, and stores `Drop` cleanup removing `path`.
       - `LocalStream::connect(path: &Path) -> io::Result<Self>`: reads `<port>\n<nonce>` from `path`, connects `TcpStream::connect(("127.0.0.1", port))`, and writes `<nonce>\n`.
       - `LocalListener::accept(&self) -> io::Result<(LocalStream, ())>`: calls `self.inner.accept()` (returning `WouldBlock` immediately in non-blocking mode), briefly reads & constant-time checks `<nonce>\n` (with a tight deadline) before returning `LocalStream`, rejecting unauthorized local connections cleanly.
     - Provide `#[cfg(windows)]` `set_nonblocking`, `set_read_timeout`, `set_write_timeout`, `try_clone`, `Read`, `Write` (plus `impl Read/Write for &LocalStream` matching `&UnixStream` in `src/view_ipc.rs:417`).

2. **Enable View, Edit, and Agent Hook Modules (`src/view_ipc.rs`, `src/agent_approval.rs`, `src/agent_hook.rs`, `src/app/host_integration.rs`)**
   - Replace `#[cfg(unix)]` on socket I/O helpers (`exchange`, `read_reply`, `read_line_by_deadline`, `send`, `run`, `edit` in `src/view_ipc.rs`; `bind`, `Pending::answer`, `accept_into`, `send_decision` in `src/agent_approval.rs`; `run_claude_code_hook` in `src/agent_hook.rs`; `prepare_view_listener` in `src/app/host_integration.rs`) with cross-platform implementations calling `LocalListener::bind` and `LocalStream::connect`.
   - Guard stale-socket cleanup (`sweep_stale_sockets`) cleanly on Windows (check PID alive via `crate::platform::process::is_pid_alive(pid)` before unlinking an abandoned `<cache>\view-<pid>-*.sock` descriptor file).
   - Confirm `ViewRequest::new` / `to_path` (`src/view_ipc.rs:90-124`) preserves arbitrary Windows paths cleanly as UTF-8 bytes.

3. **Windows `croft://` Protocol Handler (`src/cli/links.rs`)**
   - Under `#[cfg(windows)]`:
     - Implement `install_link_handler()` using `reg.exe add` under `HKCU\Software\Classes\croft` (no admin elevation required):
       - Default value `"URL:croft Protocol"`
       - `"URL Protocol" = ""`
       - `shell\open\command` default value `"\"<current_exe_dunce_path>\" open-link \"%1\""`
     - Implement `open_link(url: &str)` using `crate::platform::env::hostname()` instead of shelling out to `hostname`, and launch the target local/remote session via `Command::new(exe).status()` (matching the Windows synchronous relaunch pattern used in `src/app/mod.rs:6437-6443` instead of Unix `CommandExt::exec`).

### Verification
- Integration tests in `tests/platform.rs` for `LocalListener` non-blocking `WouldBlock`, valid `LocalStream::connect` + request/reply exchange, wrong-token rejection, and drop cleanup.
- CLI end-to-end checks on Windows: start `croft.exe .`, run `croft.exe view README.md` and `croft.exe edit --wait README.md` from its embedded PowerShell pane, and verify the tab opens and `--wait` exits cleanly on tab close.

---

## 6. Sub-project 6 — Full Unit Test Suite & Windows CI

### Problem Statement
- Currently `cargo test --test platform` passes on Windows, while `cargo test --bin croft` (covering all internal `#[cfg(test)]` unit tests across `src/`) fails to compile on `x86_64-pc-windows-msvc` due to Unix-only imports (`std::os::unix::*`, `libc::*`), `/tmp/...` paths, `#!/bin/sh` mock scripts, and an un-excluded fallback clipboard test (`src/clipboard.rs:1042-1058`).

### Architecture & Changes
1. **Gate Genuinely Unix-Only Subsystem Tests (`#[cfg(all(test, unix))]`)**
   - Audit and gate `mod tests` (or individual Unix-only test functions) in:
     - Tier-3 modules: `src/session.rs`, `src/session_host.rs`, `src/session_cli.rs`, `src/session_record.rs`, `src/pane_host.rs`, `src/web/`, `src/collab.rs`, `src/pair.rs`, `src/devcontainer.rs`, `src/gui_path.rs`.
     - Unix-specific PTY/process tests in `src/widgets/terminal/tests.rs` (`PaneLink::attach`, `UnixListener`, `libc::waitpid`), `src/dap/transport.rs` (`libc::getsid`, `libc::kill`), and `src/lsp/install.rs` (`the_node_probe_shell_runs_in_a_session_of_its_own`, Unix `chmod 000` / symlink tests).

2. **Make Shared Unit Tests Cross-Platform**
   - **`src/clipboard.rs:1042-1058` (`other_os_backend_tests`):** exclude `target_os = "windows"` (`not(any(target_os = "macos", target_os = "linux", target_os = "windows"))`) and add hermetic `#[cfg(all(test, target_os = "windows"))]` UTF-16 codec tests so `cargo test` never mutates the live Windows clipboard.
   - **Test Helper Extraction (local or per-module test helpers):**
     - Cross-platform `fake_exit_status(code: i32) -> std::process::ExitStatus` (`std::os::unix::process::ExitStatusExt` on Unix vs `std::os::windows::process::ExitStatusExt` on Windows) replacing `from_raw` in `src/lsp/install.rs:1707-1714` and similar test mocks.
     - Cross-platform `write_fake_executable(path: &Path, ...)` helper setting `0o755` on Unix and writing `.exe`/`.cmd` or file bytes on Windows.
     - Replace `/tmp/...` hardcoded paths in cross-platform tests with `tempfile::tempdir()` or `std::env::temp_dir()`.
     - Update `src/lsp/manager/tests.rs` stub-server probes (`python_for_stub_server`, line 2440) to resolve `"python"` / `"py"` on Windows in addition to `"python3"`.

3. **Windows CI Workflow (`.github/workflows/ci.yml`)**
   - Add a `windows-latest` (`x86_64-pc-windows-msvc`) build & test job alongside Unix CI running:
     - `cargo fmt --edition 2024 --check`
     - `cargo check --bin croft --tests`
     - `cargo build --bin croft`
     - `cargo test --test platform`
     - `cargo test --bin croft`

### Verification
- `cargo check --bin croft --tests` exits `0` on `x86_64-pc-windows-msvc`.
- `cargo test --test platform` and `cargo test --bin croft` pass on Windows with 0 failures.
- Unix (`macos` / `linux`) CI builds and test suites continue to pass cleanly.

---

## 7. Recommended Execution Order

1. **Sub-project 2 (PowerShell Integration & Tasks)** — highest daily UX impact for the embedded PowerShell ConPTY pane and task runner.
2. **Sub-project 3 (Host Terminal Capabilities)** — unlocks crisp Sixel/OSC 1337 graphics and full Kitty keyboard chords in Windows Terminal and WezTerm.
3. **Sub-project 4 (Windows DAP, Venvs & NTFS Save Hardening)** — finishes Python/Node/LLVM/Go debugging setup and eliminates Defender/indexer save races.
4. **Sub-project 5 (Local Windows IPC & `croft://` Links)** — enables `croft view`, `croft edit`, Claude Code approval hooks, and protocol links.
5. **Sub-project 6 (Full Unit Test Suite & Windows CI)** — locks in regression coverage across every sub-project on GitHub Actions.
