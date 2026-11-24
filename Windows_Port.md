# croft Windows Tier 2 Port — Status & Sub-project Roadmap

> **Repository:** `C:\Users\Elio\prog\croft_windows` (`github.com:Zizeo/croft_windows`, branch `windows-port`)  
> **Target Environment:** `x86_64-pc-windows-msvc`, Rust `1.97.1` (Edition `2024`), PowerShell 7+ (`pwsh.exe`), Windows PowerShell 5.1 (`powershell.exe`), `cmd.exe` fallback, hosted in Windows Terminal, WezTerm, or 3rd-party Windows terminal emulators.

---

## 1. Current Baseline (Completed through Sub-project 7 & Upstream `v2.181`)

On branch `windows-port`, **Sub-projects 1 through 7** and the **upstream `v0.2.160`–`v0.2.181` (`v2.181`) merge** are complete and verified:

1. **Sub-project 1 — Compilable Core & Platform Layer:**
   - Relocated `libc = "0.2"` and `async-lsp`'s `"stdio"` feature to `[target.'cfg(unix)'.dependencies]`; added `dunce = "1"` and minimal `windows-sys = "0.59"`.
   - Replaced the `date` subprocess in `build.rs` with a pure-Rust UTC ISO-8601 formatter.
   - Built unified OS abstractions under `src/platform/` (`env`, `fs`, `process`, `time`, `clipboard`, `os`, `ipc`) with `unsafe` Win32 FFI confined strictly to `src/platform/`, plus `dunce::canonicalize` UNC stripping, Win32 Job Objects (`ManagedChild`, `TreeKiller`), and Win32 `CF_UNICODETEXT` clipboard support.
   - Added ConPTY channel-bridged PTY reading in `src/widgets/terminal/backend.rs` and `src/widgets/terminal/spawn.rs`.
2. **Windows LSP Discovery & Provisioning (`95018c0`):**
   - Added `.exe`/`.cmd`/`.bat` executable lookup (`src/platform/env.rs`, `src/lsp/manager/discovery.rs`), Windows `uv.exe` PowerShell bootstrap and Node/`npm.cmd` discovery (`src/lsp/install.rs`), and `windows-x86_64` binary targets/digests for `rust-analyzer`, `taplo`, `clangd`, `marksman`, and `csvlens`.
3. **Windows SSH Workspaces (`a66ba1f`):**
   - Ported `croft remote`, `croft sync-config`, and `SshAuth` (`src/remote/control.rs`, `src/remote_connect.rs`, `src/remote_bulk.rs`, `src/remote/*.rs`) to socket-free Windows OpenSSH (`ssh.exe`) and tracked `ssh -N -L` port forwards (`src/remote/forward.rs`).
4. **Sub-project 2 — PowerShell Integration & Task Runner (`45c422f`):**
   - Implemented `shellIntegration.ps1` (`OSC 133` + `OSC 7` + Windows drive-letter URL normalization) and PowerShell/cmd/Git Bash/WSL profile discovery, plus `ShellFlavor`-aware quoting and location/env wrappers in `src/tasks.rs`.
5. **Sub-project 3 — Host Terminal Capabilities (`9f6fdac`):**
   - Implemented Win32 console capability probing (`src/platform/terminal.rs`) for DA1 Sixel, Kitty keyboard protocol, and `CSI 16 t`/`14 t` + `GetCurrentConsoleFontEx` cell pixel geometry in Windows Terminal and WezTerm.
6. **Sub-project 4 — Windows Toolchains & NTFS Hardening (`c8f6ab4`):**
   - Wired Windows DAP venvs (`Scripts\python.exe`), `dlv.exe`, `lldb-dap.exe`, `node_modules\.bin\tsc.cmd`, and `rename_retrying`/`write_retrying` for Defender/indexer sharing violations.
7. **Sub-project 5 — Local Windows IPC & `croft://` Links (`ce7458c`):**
   - Implemented capability-authenticated loopback TCP `LocalListener`/`LocalStream` (`src/platform/ipc.rs`) enabling `croft view`, `croft edit --wait`, and Claude Code approval hooks (`croft hook`), plus `HKCU\Software\Classes\croft` link registration (`src/cli/links.rs`).
8. **Sub-project 6 — Full Unit/CLI Test Suites & Windows CI/Release (`616d850`, `7088d88`):**
   - Enabled full `--test platform`, `--test cli`, and `--bin croft` CI checks on `windows-latest` (`.github/workflows/ci.yml`) and `x86_64-pc-windows-msvc` release artifacts (`.github/workflows/release.yml`).
9. **Upstream `v0.2.160`–`v0.2.181` Merge (`0fd612d`, `9dd26b4`):**
   - Merged all 22 upstream releases from `v2.181` while preserving `TreeKiller`/`ManagedChild` process-tree cancellation, added `is_pid_alive` sweeping for stale decompressed gzip temp copies (`src/widgets/editor/viewers.rs`), added `WT_SESSION` SGR 58 underline-color support (`src/color_depth.rs`), and adapted new popup/tick/git/testing-worker tests for Windows.
10. **Sub-project 7 — Windows Feature Parity & Polish (`ffd064d`..`cc1f2b6`):**
   - Enabled "Reveal in File Explorer" in Explorer/tab menus and shortcuts (`src/platform/os.rs`, `src/app/*`), shell-aware `format_cd_command_for` (`PowerShell`/`Cmd`/`Posix`), prebuilt `x86_64-pc-windows-msvc` self-update + `.old` swap (`src/update_check.rs`, `src/remote_prebuilt.rs`), "Move/Copy Tab into New Window" for Windows Terminal / WezTerm (`src/app/editor_tabs.rs`), native Win32 TCP port detection & auto-retirement (`src/platform/net.rs`, `src/port_detect.rs`), ConPTY descendant foreground resolution for pane labels, agents & SSH offers (`src/widgets/terminal/processes.rs`, `src/app/terminal_panes.rs`), zoxide Windows discovery, `winget` install & PowerShell profile hook (`src/zoxide.rs`), and background `stderr` draining + `.exe` resolution in PDF rendering (`src/pdf.rs`).

*(Tier-3 Unix-daemon / OpenSSH-`ControlMaster`-dependent features — `attach`, `ls`, `record`, `web`, `pane-host`, `session-host`, `collab`, `pair`, `devcontainer` — remain out of native Windows scope and direct users to WSL2.)*

---

## 2. Sub-project 2 — PowerShell Shell Integration (`OSC 133` / `OSC 7`) & Task Runner (Completed)

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

## 3. Sub-project 3 — Host Terminal Capabilities (Windows Terminal & WezTerm) (Completed)

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

## 4. Sub-project 4 — Windows Toolchains (DAP, Venvs, Task Binaries, & NTFS Save Hardening) (Completed)

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

## 5. Sub-project 5 — Local Windows IPC (`croft view`, `croft edit`, `croft hook`) & `croft://` Links (Completed)

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

## 6. Sub-project 6 — Full Unit Test Suite & Windows CI (Completed)

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

## 7. Sub-project 7 — Windows Feature Parity & Polish (Completed)

A full audit of `croft` on `windows-port` (`9dd26b40d`) identified **8 Tier-2 user-facing features** that are either still macOS/Unix-gated, fall back to no-ops on Windows, or fail because of Windows executable/pipe/shell differences:

### 1. Prebuilt Self-Update & Running `.exe` Replacement (`src/update_check.rs`, `src/remote_prebuilt.rs`)
- **Problem Statement:**
  - `prebuilt_triple_for("windows", "x86_64")` (`src/update_check.rs:530-540`) returns `None`, even though `.github/workflows/release.yml` publishes `croft-x86_64-pc-windows-msvc.tar.gz`. Help → Update falls back to compiling from source (`cargo install`) instead of downloading the signed prebuilt release binary.
  - `unpack_binary(archive, triple)` (`src/remote_prebuilt.rs:171-173`) looks strictly for `croft-{triple}/croft`, missing `croft-{triple}/croft.exe` inside Windows release archives.
  - `apply_staged_at(staged, target, tmp)` (`src/update_check.rs:831-846`) calls `std::fs::rename(tmp, target)`. On Windows, `MoveFileExW` cannot overwrite a currently executing `croft.exe` (it fails with `ERROR_ACCESS_DENIED`), even though Windows *does* allow renaming the running `.exe` to a sibling backup name (`croft.exe.old`).
- **Architecture & Changes:**
  - Map `("windows", "x86_64")` -> `Some("x86_64-pc-windows-msvc")` in `prebuilt_triple_for` (and update `src/update_check.rs:1464`).
  - In `src/remote_prebuilt.rs:unpack_binary`, expect `croft-{triple}/croft.exe` when `triple.contains("-windows-")`.
  - Under `#[cfg(windows)]` in `apply_staged_at`: rename an existing `target` to `<target>.old` (removing any stale `.old` first), move `tmp` into `target`, and sweep leftover `croft.exe.old` files on startup once the relaunched process is running.

### 2. Enable "Reveal in File Explorer" in Explorer & Tab Menus (`src/app/explorer.rs`, `src/app/editor_tabs.rs`, `src/app/editor_input.rs`)
- **Problem Statement:**
  - `crate::platform::os::reveal_in_file_manager` (`src/platform/os.rs:23-39`) already implements `explorer.exe /select,<path>`, and `App::reveal_in_finder` (`src/app/editor_tabs.rs:1935`) delegates to it.
  - However, all UI entry points remain gated to `cfg!(target_os = "macos")` and labeled `"Reveal in Finder"`:
    - Explorer context menu (`src/app/explorer.rs:306-336`)
    - Explorer keyboard shortcut (`src/app/explorer.rs:2405`)
    - Editor tab context menu (`src/app/editor_tabs.rs:50-58`)
    - Editor keyboard shortcut (`src/app/editor_input.rs:519-522`)
- **Architecture & Changes:**
  - Enable `reveal_enabled` on local Windows sessions (`!self.is_remote && cfg!(any(target_os = "macos", target_os = "windows"))`).
  - Display `"Reveal in Finder"` on macOS and `"Reveal in File Explorer"` on Windows across both context menus and command labels.

### 3. "Move / Copy Tab into New Window" (`src/app/editor_tabs.rs:1941-2118`)
- **Problem Statement:**
  - `spawn_new_window` (`src/app/editor_tabs.rs:2109-2118`) returns `Unsupported("opening a new window is only supported on macOS")`, disabling "Move into New Window" and "Copy into New Window" on Windows.
- **Architecture & Changes:**
  - Add a `#[cfg(windows)]` `spawn_new_window(root, file)` implementation selecting the host terminal:
    - Windows Terminal (`WT_SESSION` set): `wt.exe -w new <exe> <root> --open-file <file> --zen`
    - WezTerm (`TERM_PROGRAM=WezTerm`): `wezterm start -- <exe> <root> --open-file <file> --zen`
    - Fallback (`conhost` / other): `cmd.exe /C start "" <exe> <root> --open-file <file> --zen`

### 4. Native Port Detection & Auto-Retirement (`src/port_detect.rs:478-507`)
- **Problem Statement:**
  - `poll_listeners` returns `Vec::new()` and `poll_all_listening` returns `None` on `#[cfg(not(unix))]`.
  - As a result, dev servers that bind a socket silently without printing a URL to stdout/stderr are never discovered in the Ports panel, and stopped servers are never auto-retired by socket polling.
- **Architecture & Changes:**
  - Add a Win32 IP Helper backend (`GetExtendedTcpTable` for `AF_INET` and `AF_INET6` `TCP_TABLE_OWNER_PID_LISTENER`, confined to `src/platform/` or `#[cfg(windows)]` helper) to enumerate listening loopback/wildcard TCP ports and their owning PIDs.
  - Match owning PIDs against the terminal shell's descendant PID tree using `sysinfo` (`Process::parent()`) so `poll_listeners` and `poll_all_listening` work natively without `ss` or `lsof`.

### 5. Live Terminal Pane Auto-Labels, Agent Detection & SSH Offer (`src/widgets/terminal/processes.rs`, `src/app/terminal_panes.rs:1865-1936`)
- **Problem Statement:**
  - `PaneBackend::process_group_leader()` (`src/widgets/terminal/backend.rs`) returns `None` on Windows because ConPTY does not expose POSIX `tcgetpgrp`.
  - Consequently `refresh_terminal_labels` (`src/app/terminal_panes.rs:1865-1936`) never sees a foreground child PID, leaving pane tab labels stuck on the shell name instead of updating to running tools (`cargo`, `python`, `claude`, `ssh`), disabling active agent detection, and preventing the `#364` "Open SSH workspace" prompt when running `ssh host` inside a pane.
- **Architecture & Changes:**
  - When `process_group_leader()` is `None` on Windows and `shell_pid` is known, walk the live child-process tree rooted at `shell_pid` via `sysinfo` (skipping `conhost.exe` / `OpenConsole.exe` host shims) to resolve the deepest active foreground child PID for `process_name` and `process_cmdline`.

### 6. Shell-Aware `format_cd_command` & Line-Clear Sequence (`src/widgets/terminal/processes.rs:412-435`)
- **Problem Statement:**
  - `CLEAR_TYPED_LINE` (`\x05x\x15`) uses Emacs/readline `Ctrl+E` + `Ctrl+U`, and `format_cd_command(path)` emits POSIX `cd '...'` with `'\''` apostrophe escaping.
  - On PowerShell (where `Esc` clears the line in PSReadLine and `''` escapes single quotes) or `cmd.exe` (which requires `cd /d "..."`), paths containing apostrophes or spaces (or pre-typed text in PowerShell) can fail when croft retargets a pane's working directory.
- **Architecture & Changes:**
  - Parameterize `format_cd_command` by `ShellFlavor` (reusing the flavor logic from `src/tasks.rs`):
    - `PowerShell`: `\x1bSet-Location '<escaped_with_doubled_quotes>'\r`
    - `Cmd`: `\x1bcd /d "<path>"\r`
    - `Posix`: keep `\x05x\x15cd '...'\n` unchanged.

### 7. Zoxide Jump (`Ctrl+Z`) Windows Installer, Discovery & PowerShell Hook (`src/zoxide.rs`)
- **Problem Statement:**
  - `zoxide::query` works on Windows if `zoxide.exe` is already on `PATH`, but:
    - `fallback_paths()` (`src/zoxide.rs:144-153`) checks only `~/.local/bin/zoxide`, `/opt/homebrew/bin/zoxide`, `/usr/local/bin/zoxide`, and `/usr/bin/zoxide` (missing `.exe`, `%USERPROFILE%\.cargo\bin\zoxide.exe`, and `%LOCALAPPDATA%\Microsoft\WinGet\Links\zoxide.exe`).
    - `install_in_background()` (`src/zoxide.rs:472`) spawns `sh -c "curl ... | sh"`.
    - `supported_shell()` / `ensure_shell_init()` (`src/zoxide.rs`) supports only `bash` and `zsh`, skipping PowerShell (`$PROFILE` with `Invoke-Expression (& { (zoxide init powershell --cmd j | Out-String) })`).
- **Architecture & Changes:**
  - Add Windows `.exe` fallback paths and `winget` / PowerShell install script support in `src/zoxide.rs`.
  - Add PowerShell profile hook injection (`zoxide init powershell --cmd j`) guarded by the same managed sentinel markers (`# >>> croft zoxide (managed) >>>`).

### 8. PDF Renderer Non-Blocking Pipe Reader (`src/pdf.rs:562-591`)
- **Problem Statement:**
  - `set_nonblocking(pipe: Option<&ChildStderr>)` returns `false` on `#[cfg(not(unix))]`, skipping incremental `stderr` draining during the render loop.
  - If `pdftoppm.exe` or `mutool.exe` emits more than a Windows anonymous pipe buffer (~4 KB) on `stderr` before exiting, the child can stall until the render timeout kills it.
- **Architecture & Changes:**
  - Under `#[cfg(windows)]` in `src/pdf.rs`, spawn a small bounded reader thread to drain `ChildStderr` into an in-memory buffer (`Vec<u8>`) concurrently with `try_wait()`, preventing pipe-buffer deadlock while keeping the same bounded render timeout.

### Verification
- Unit tests for:
  - `prebuilt_triple_for("windows", "x86_64")`, `unpack_binary` on `croft-x86_64-pc-windows-msvc`, and `apply_staged_at` `.old` rename-aside behavior on Windows.
  - `spawn_new_window` argv generation for `wt.exe` and `wezterm`.
  - `port_detect` Win32 TCP listener table parsing / descendant PID filtering.
  - `format_cd_command` across `PowerShell`, `Cmd`, and `Posix`.
  - `zoxide` Windows fallback paths and PowerShell `$PROFILE` hook injection.
- Manual smoke test in Windows Terminal: verify Help → Update, Explorer/Tab "Reveal in File Explorer", "Move into New Window", live pane labels, and Ports panel auto-detection/retirement.

---

## 8. Intentionally Out-of-Scope (Tier-3 / WSL2) & Platform-Specific Features

The following subsystems in upstream `croft` are **deliberately excluded** from native Windows (or inherently tied to another OS):

1. **Tier-3 Unix-Daemon / Socket Features (directed to WSL2):**
   - Persistent sessions (`croft attach`, `croft ls`, `session_host.rs`, `session_cli.rs`, `session.rs`) and `dtach` reattach
   - Session recording (`croft record`, `session_record.rs`)
   - Local pane daemon (`pane_host.rs`)
   - Remote web UI (`croft web`, `src/web/`)
   - Collaborative editing & pairing (`src/collab.rs`, `src/collab_agent.rs`, `src/pair/`, `src/pair_host.rs`)
   - Devcontainers (`src/devcontainer.rs` — currently stubbed out on Windows; could optionally be revisited later on top of Docker Desktop)
   - OpenSSH `ControlMaster` socket multiplexing (`ssh_control_dir`, `cross_build_unavailable` — Windows uses direct `ssh.exe` + tracked `ssh -N -L` forwards)
2. **OS-Specific Integrations (non-Windows by design):**
   - `croft setup-terminal` (Apple Terminal.app), `croft setup-iterm2` (iTerm2 plist), and `croft setup-ghostty` (macOS Ghostty Cmd-chord forwarding; a `croft setup-wt` helper for Windows Terminal is an optional future enhancement, not an upstream parity gap)
   - `src/gui_path.rs` (macOS launchd PATH capture)
   - `src/voice.rs` (Android Termux:API `termux-dialog speech` dictation)
   - Bundled `lua-language-server` (`assets/extensions/lsp-lua/extension.toml` expects `lua-language-server` on `PATH` on all platforms, including Windows)

---

## 9. Recommended Execution Order for Sub-project 7

1. **Item 2 ("Reveal in File Explorer") & Item 6 (`format_cd_command` for PowerShell/cmd)** — quick, self-contained UX wins with existing platform primitives.
2. **Item 1 (Prebuilt Self-Update & `.exe.old` swap) & Item 3 ("Move/Copy into New Window" for Windows Terminal / WezTerm)** — completes desktop app lifecycle parity on Windows.
3. **Item 4 (Win32 TCP Port Detection) & Item 5 (ConPTY Descendant Process Pane Labels / SSH Offer)** — restores live Ports panel and terminal pane intelligence.
4. **Item 7 (Zoxide Windows + PowerShell Hook) & Item 8 (PDF `stderr` Reader Thread)** — eliminates the remaining tool-integration edge cases.
