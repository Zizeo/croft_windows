# croft Windows Tier 2 Port — Sub-project 1: Compilable Core & Platform Layer

**Date:** 2026-11-22
**Branch:** `windows-port`
**Status:** Draft for review

## Context & Tier 2 Roadmap

croft is a ~522k-LOC Rust TUI IDE built on cross-platform crates (`ratatui`, `crossterm`, `alacritty_terminal`, `portable-pty`, `tree-sitter`, `tokio`, `notify`, `rusqlite`, `resvg`, `sysinfo`, `trash`), but it currently fails to compile on Windows because of direct `libc` syscalls (199 hits / 47 production files), `std::os::unix` imports (195 hits / 74 production files), Unix domain sockets (111 hits / 20 production files), POSIX shell assumptions, and Unix-only host terminal probes.

The full Tier 2 native Windows + PowerShell port (targeting Windows Terminal, WezTerm, and any 3rd-party Windows terminal emulator) is decomposed into six sequential sub-projects, each with its own spec, implementation plan, and verifiable deliverable:

1. **Sub-project 1 (this spec) — Compilable Windows Core & Platform Layer (`src/platform/`):** Clean `x86_64-pc-windows-msvc` build; unified `src/platform/` OS abstraction; Windows paths & `dunce` canonicalization; Win32 clipboard & OS opener; module-boundary gating of Tier-3 Unix-only subsystems; minimal working ConPTY reader loop with PowerShell fallback.
2. **Sub-project 2 — ConPTY Terminal & PowerShell Integration:** Shell discovery/profiles (`pwsh.exe`, `powershell.exe`, `cmd.exe`), `shellIntegration.ps1` (OSC 133 `A/B/C/D` + OSC 7 CWD for PowerShell 5.1 & 7+), symlink-free `pane-bin`, and PowerShell-aware quoting/env generation in `src/tasks.rs`.
3. **Sub-project 3 — Host Terminal Compatibility (Windows Terminal, WezTerm & 3rd-Party Emulators):** Windows stdin DA1 & Kitty keyboard probe (enabling Sixel in Windows Terminal 1.22+ and Kitty keyboard chords in WezTerm), CSI `16t`/`14t` + Win32 cell-pixel-size query replacing `crossterm::terminal::window_size()`, and clean fallback on `conhost.exe` / Alacritty.
4. **Sub-project 4 — Windows Toolchains (LSP, DAP, Python/Node, NTFS Save Safety):** `.exe`/`.cmd`/`PATHEXT` discovery, `.venv\Scripts\python.exe`, `node_modules\.bin\*.cmd`, Windows binary targets in `lsp/install.rs` and `dap/install.rs`, and NTFS file-locking/atomic-save hardening.
5. **Sub-project 5 — Local Windows IPC (`croft view`, `croft edit`, `croft hook`):** Named Pipes / Windows `AF_UNIX` implementation behind `src/platform/ipc.rs` for `view_ipc`, `agent_approval`, and `agent_hook`.
6. **Sub-project 6 — Windows Test Harness & CI:** Cross-platform test helpers (replacing `/tmp`, `#!/bin/sh` mocks, Unix symlinks, `PermissionsExt`) and Windows CI workflow.

## Global Constraints

- **Toolchain:** Rust `1.97.1`, Edition `2024` (`rust-toolchain.toml`, `Cargo.toml`).
- **Targets:** Must compile and run on `x86_64-pc-windows-msvc` while preserving full `#[cfg(unix)]` compatibility (Linux and macOS code paths must remain intact).
- **Shells & Terminals:** Must support PowerShell 7+ (`pwsh.exe`), Windows PowerShell 5.1 (`powershell.exe`), and `cmd.exe` fallback, hosted in Windows Terminal, WezTerm, or any 3rd-party terminal emulator (with text-glyph fallback on legacy `conhost.exe`).
- **Dependencies:**
  - Move `libc = "0.2"` to `[target.'cfg(unix)'.dependencies]` so unguarded `libc::` references fail fast at compile time on Windows.
  - Move `async-lsp`'s `"stdio"` feature to `[target.'cfg(unix)'.dependencies]` (keep `["omni-trait", "tracing", "tokio"]` cross-platform; croft does not use `async_lsp::stdio`).
  - Add only `windows-sys = "0.59"` (minimal feature set) and `dunce = "1"` under `[target.'cfg(windows)'.dependencies]`.
  - Use Rust stdlib `std::fs::File::lock` / `File::try_lock` (available since Rust 1.89) instead of adding an external file-locking crate.
- **Safety boundary:** All `unsafe` Win32 FFI must live exclusively inside `src/platform/` and `src/clipboard.rs` — zero direct `windows_sys` calls in `src/app/` or `src/widgets/`.
- **CLI parity:** Unsupported Tier-3 subcommands remain registered in `clap` (`src/cli/commands.rs`) and return a clear, actionable runtime error on Windows rather than being removed from `--help`.

## Sub-project 1 Goals & Non-Goals

### In Scope
- `cargo build --bin croft` compiles cleanly on `x86_64-pc-windows-msvc` and produces `target/debug/croft.exe`.
- Pure-Rust UTC build timestamp in `build.rs` (no Unix `date` subprocess).
- Unified `src/platform/` layer replacing direct `libc` / `std::os::unix` calls across production modules.
- Native Windows config/state/cache directories (`%APPDATA%\croft`, `%LOCALAPPDATA%\croft\state`, `%LOCALAPPDATA%\croft\cache`) while honoring explicit `XDG_CONFIG_HOME` / `XDG_STATE_HOME` overrides.
- Clean `dunce::canonicalize` across production path resolution (no `\\?\` verbatim UNC prefixes leaking to UI, Git, or LSPs).
- Native Win32 clipboard (`CF_UNICODETEXT`) in `src/clipboard.rs` and `explorer.exe /select,` reveal helper.
- Clean module-boundary Windows stubs for Tier-3 Unix-only subsystems and temporary stubs for local IPC (ready for Sub-project 5).
- Minimal working local ConPTY terminal reader in `src/widgets/terminal/backend.rs` and Windows PowerShell default shell fallback in `src/widgets/terminal/spawn.rs` so `App::new()` launches with a working basic PowerShell pane.

### Explicitly Out of Scope (Deferred to Later Sub-projects or Tier 3)
- PowerShell OSC 133/7 shell integration, terminal profile discovery, symlink-free `pane-bin`, and `tasks.rs` PowerShell quoting → **Sub-project 2**.
- Windows console DA1 / Kitty keyboard probe and cell-pixel-size query for inline graphics → **Sub-project 3**.
- Windows LSP/DAP provisioning URLs, `.venv\Scripts\python.exe`, `.cmd` wrappers, and NTFS save stress-hardening → **Sub-project 4**.
- Local Windows IPC (`croft view`, `croft edit`, `croft hook`) → **Sub-project 5**.
- Porting the full 9,234-test suite and adding GitHub Actions Windows CI → **Sub-project 6**.
- Tier-3 Unix-socket/SSH features (`croft attach`, `ls`, `record`, `web`, `pane-host`, `collab`, `pair`, `remote`, `sync-config`, `devcontainer`, `execve` binary hot-swap) → **permanently out of Tier 2 scope**.

## Architecture

### 1. Build & Dependencies (`Cargo.toml`, `build.rs`)

- **`Cargo.toml`:**
  - Relocate `libc = "0.2"` under `[target.'cfg(unix)'.dependencies]`.
  - Split `async-lsp` features: base `["omni-trait", "tracing", "tokio"]` in `[dependencies]`, plus `features = ["stdio"]` under `[target.'cfg(unix)'.dependencies]`.
  - Add `[target.'cfg(windows)'.dependencies]`:
    - `dunce = "1"`
    - `windows-sys = { version = "0.59", features = ["Win32_Foundation", "Win32_System_DataExchange", "Win32_System_JobObjects", "Win32_System_Memory", "Win32_System_SystemInformation", "Win32_System_Threading", "Win32_UI_Shell"] }`
- **`build.rs`:**
  - Replace `Command::new("date")` in `emit_build_time()` with a pure-Rust civil-from-days UTC formatter over `SystemTime::now().duration_since(UNIX_EPOCH)`, emitting `YYYY-MM-DDTHH:MM:SSZ` on every OS with zero subprocesses or extra build dependencies.

### 2. Platform Abstraction (`src/platform/`)

All recurring OS primitives live in `src/platform/` (`mod.rs` + 6 focused submodules), each providing identical cross-platform signatures backed by `#[cfg(unix)]` (unchanged behavior) and `#[cfg(windows)]`:

#### `src/platform/env.rs`
- `pub fn home_dir() -> Option<PathBuf>`:
  - Unix: `HOME`.
  - Windows: `HOME` (if set and non-empty) → `USERPROFILE` → `HOMEDRIVE` + `HOMEPATH`.
- `pub fn config_dir() -> PathBuf`, `state_dir() -> PathBuf`, `cache_dir() -> PathBuf`:
  - Unix: existing `XDG_CONFIG_HOME` / `XDG_STATE_HOME` / `HOME` logic from `src/paths.rs`.
  - Windows: honor explicit absolute/non-empty `XDG_CONFIG_HOME` / `XDG_STATE_HOME` first; otherwise `%APPDATA%\croft` (config), `%LOCALAPPDATA%\croft\state` (state), `%LOCALAPPDATA%\croft\cache` (cache), falling back to `home_dir()` before `.`.
- `pub fn hostname() -> String`:
  - Unix: `libc::gethostname`.
  - Windows: `GetComputerNameExW(ComputerNameDnsHostname)` falling back to `COMPUTERNAME` env var or `"local"`.
- `pub fn default_shell() -> String`:
  - Unix: `$SHELL` → `"/bin/bash"`.
  - Windows: resolve `pwsh.exe` on `PATH` → `powershell.exe` on `PATH` → `%COMSPEC%` → `"cmd.exe"`.

All ~46 production call sites reading `std::env::var_os("HOME")` are updated to call `crate::platform::env::home_dir()`.

#### `src/platform/fs.rs`
- `pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf>`:
  - Unix: `std::fs::canonicalize(path)`.
  - Windows: `dunce::canonicalize(path)` (strips `\\?\` verbatim UNC prefix when safe).
- `pub fn is_executable(path: &Path) -> bool`:
  - Unix: `meta.is_file() && (meta.permissions().mode() & 0o111 != 0)`.
  - Windows: `meta.is_file()` and file extension matches an entry in `%PATHEXT%` (defaulting to `.EXE`, `.CMD`, `.BAT`, `.COM`, case-insensitive).
- `pub fn set_private_file_mode(opts: &mut OpenOptions)` & `pub fn set_private_dir_mode(builder: &mut DirBuilder)`:
  - Unix: `0o600` / `0o700`.
  - Windows: no-op (inherits user-profile ACL).
- `pub fn open_nofollow(opts: &mut OpenOptions)`:
  - Unix: `custom_flags(libc::O_NOFOLLOW)`.
  - Windows: no-op (`OpenOptions::create_new` already prevents following pre-existing targets; reparse-point check helper provided where needed for `hot_exit` / `config_layers`).
- `pub fn try_lock_exclusive(file: &File) -> std::io::Result<bool>` & `pub fn lock_exclusive(file: &File) -> std::io::Result<()>`:
  - Cross-platform via Rust 1.97 `File::try_lock()` / `File::lock()` (mapping `TryLockError::WouldBlock` to `Ok(false)`).
- `pub fn create_file_link(target: &Path, link: &Path) -> std::io::Result<()>`:
  - Unix: `std::os::unix::fs::symlink(target, link)`.
  - Windows: `std::fs::hard_link(target, link)` with `std::fs::copy(target, link)` fallback if across volumes — never requires Admin or Developer Mode privileges.

#### `src/platform/process.rs`
- `pub fn detach_from_tty(cmd: &mut Command)`:
  - Unix: `pre_exec(|| { libc::setsid(); Ok(()) })`.
  - Windows: `creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)`.
- `pub struct ManagedChild` (wraps `std::process::Child` + optional Windows Job Object handle):
  - `pub fn spawn_in_group(cmd: &mut Command) -> std::io::Result<ManagedChild>`:
    - Unix: `process_group(0)` + `spawn()`.
    - Windows: `creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)` + `spawn()`, then creates an anonymous Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and calls `AssignProcessToJobObject`. If job assignment fails (e.g., restricted nested job), keeps the `Child` without failing the spawn and falls back to `Child::kill()` on teardown.
  - `pub fn kill_tree(&mut self)`:
    - Unix: `kill(-pid, SIGKILL)` + `wait()`.
    - Windows: `TerminateJobObject(job, 1)` (or `child.kill()` fallback) + `wait()`.
- `pub fn is_pid_alive(pid: u32) -> bool`:
  - Unix: `kill(pid, 0) == 0 || errno == EPERM`.
  - Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)` + `GetExitCodeProcess == STILL_ACTIVE` (`259`), closing the handle on drop.
- `pub fn terminate_pid(pid: u32) -> bool`:
  - Unix: `kill(pid, SIGTERM) == 0`.
  - Windows: `OpenProcess(PROCESS_TERMINATE, 0, pid)` + `TerminateProcess(handle, 1)`.
- `pub fn raise_fd_limit()` & `pub fn reset_sigpipe_default()`:
  - Unix: existing `getrlimit`/`setrlimit` and `signal(SIGPIPE, SIG_DFL)`.
  - Windows: no-ops.

#### `src/platform/time.rs`
- `pub struct LocalDateTime { pub year: i32, pub month: u32, pub day: u32, pub hour: u32, pub minute: u32, pub second: u32 }`
- `pub fn local_datetime(unix_secs: i64) -> Option<LocalDateTime>`:
  - Unix: `libc::localtime_r`.
  - Windows: convert Unix seconds to Windows `FILETIME` (100 ns intervals since 1601-01-01), then `FileTimeToLocalFileTime` + `FileTimeToSystemTime`.
- `pub fn thread_cpu_time() -> Option<Duration>`:
  - Unix: `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`.
  - Windows: `GetThreadTimes(GetCurrentThread(), ...)` summing kernel + user time.

#### `src/platform/os.rs` & `src/clipboard.rs`
- `src/platform/os.rs`: `reveal_in_file_manager(path: &Path) -> Result<()>` uses `explorer.exe /select,<path>` on Windows (macOS keeps `open -R`).
- `src/clipboard.rs`: Add `#[cfg(windows)] mod windows`:
  - `read_string() -> Option<String>` and `write_string(text: &str) -> bool` using Win32 `OpenClipboard` (retrying up to 5 times with 10 ms sleep on transient lock contention), `EmptyClipboard`, `GetClipboardData`/`SetClipboardData` with `CF_UNICODETEXT`, UTF-16LE NUL-terminated encoding, and RAII drop guards for `CloseClipboard` and `GlobalUnlock`.
  - Pure UTF-16LE codec helpers (`encode_utf16_nul` / `decode_utf16_nul`) exposed for unit testing without touching the OS clipboard.

#### `src/platform/ipc.rs`
- Unix: re-exports `LocalListener = std::os::unix::net::UnixListener` and `LocalStream = std::os::unix::net::UnixStream`.
- Windows (Sub-project 1): inert placeholder types (`LocalListener`, `LocalStream`) so `App` fields (`view_listener`, `hook_listener`) and method signatures compile unchanged. `App::bind_view_socket()` and `agent_approval::bind()` silently return `None` at startup (no error toast on launch), while `croft view`, `croft edit`, and `croft hook` CLI commands return a clear unsupported error until Sub-project 5 implements Windows Named Pipes / `AF_UNIX`.

### 3. Module-Boundary Gating for Tier-3 Subsystems

Rather than scattering `#[cfg(unix)]` throughout `src/app/`, each Tier-3 module compiles on Windows via a thin module-level stub:
- **CLI entry points** (`attach`, `ls`, `record`, `web`, `pair`, `collab-relay`, `collab-agent`, `pane-host`, `session-host`, `remote`, `sync-config`, `devcontainer`, `open-link`, `install-link-handler`): stay in `CliCommand` and return `anyhow::bail!("croft <cmd> is not supported on native Windows; run croft in WSL2 for this feature")`.
- **App-integrated helpers:**
  - `gui_path::repair()` → no-op on Windows.
  - `collab` / `pair` → inert Windows structs/no-op tick methods so `App` stays clean.
  - `port_detect` → keeps its cross-platform PTY output scraper; gates only the `lsof`/`ss` socket poller to `#[cfg(unix)]`.
  - `app/mod.rs` self-update relaunch (`pending_reexec`) → spawns the target binary via `Command::spawn()` and exits on Windows instead of calling Unix `CommandExt::exec()`.
  - `StartupProbe` (`app/mod.rs:6148`) → stays `#[cfg(unix)]` in Sub-project 1 (replaced in Sub-project 3).

### 4. Minimal ConPTY Terminal Backend (`src/widgets/terminal/`)

`App::new()` calls `PtyTerminal::new(&root)?` unconditionally (`src/app/mod.rs:4225`), so Sub-project 1 provides a minimal working ConPTY reader:
- `spawn.rs`: `PtyTerminal::new` and `new_with_transcript` call `crate::platform::env::default_shell()`.
- `backend.rs`:
  - `PaneBackend::Hosted` and `PaneLink::attach` are gated to `#[cfg(unix)]` (`persistent_panes_enabled()` is already `false` on Windows).
  - Under `#[cfg(unix)]`, `poll_fd` + `libc::poll` + shutdown pipe remain unchanged.
  - Under `#[cfg(windows)]`: `PaneLink::local` spawns ConPTY via `portable-pty` without calling `as_raw_fd()`. A background reader thread calls `reader.read(&mut buf)` and sends chunks over an `mpsc::Sender<Vec<u8>>` to the terminal parser loop, which uses `recv_timeout` to service `PORT_URL_SETTLE_MS` deadlines without `libc::poll`.
  - On `Drop for PtyTerminal`: `backend.end()` terminates the child and drops `MasterPty` (triggering `ClosePseudoConsole` and closing the ConPTY output pipe, which unblocks the reader thread). Joining the reader thread uses a short bounded wait so a leaked grandchild handle can never freeze croft on exit.
  - `process_group_leader()` returns `None` on Windows (Sub-project 2 wires PowerShell OSC 7/133 for live CWD and command tracking).

## Error Handling & Edge Cases

- **Transient clipboard locks:** Retry `OpenClipboard` up to 5 × 10 ms before returning `false`/`None` (and fall back to OSC 52 on copy, matching existing `clipboard.rs` semantics).
- **Embedded NULs in clipboard:** Rejected before FFI, matching macOS behavior.
- **Job Object nesting failures:** `AssignProcessToJobObject` failure logs/ignores gracefully and falls back to `Child::kill()` instead of aborting child spawns.
- **Windows symlink privilege errors (`ERROR_PRIVILEGE_NOT_HELD`):** Eliminated by `platform::fs::create_file_link` using hard link + copy fallback.
- **Verbatim UNC paths (`\\?\C:\...`):** Eliminated by routing production `canonicalize` calls through `platform::fs::canonicalize` (`dunce::canonicalize`).
- **Missing `HOME` env var:** Eliminated by `platform::env::home_dir()` falling back to `USERPROFILE` / `HOMEDRIVE`+`HOMEPATH`, and `config_dir`/`state_dir`/`cache_dir` using `%APPDATA%` / `%LOCALAPPDATA%`.

## Verification Plan

1. **Automated unit tests (`cargo test --lib platform::` and `cargo test --lib clipboard::`):**
   - `platform::env`: `XDG_*` override vs `%APPDATA%`/`%LOCALAPPDATA%`, `home_dir()` fallback chain, non-empty `hostname()`, `default_shell()` finds an installed shell.
   - `platform::fs`: `canonicalize` returns standard `C:\...` path without `\\?\`, `is_executable` accepts `.exe`/`.cmd`/`.bat` and rejects `.txt`, `try_lock_exclusive` enforces mutual exclusion across two `File` handles, `create_file_link` succeeds without elevation.
   - `platform::process`: `is_pid_alive(std::process::id()) == true`, returns `false` after child exit/kill; `ManagedChild::kill_tree` terminates a parent + child process cleanly.
   - `platform::time`: `local_datetime` returns valid fields for a known epoch timestamp; `thread_cpu_time()` returns `Some`.
   - `clipboard`: UTF-16LE codec round-trip tests (ASCII, accented characters, emoji surrogate pairs, NUL rejection).
2. **Build & CLI checks:**
   - `cargo build --bin croft` succeeds with zero errors on `x86_64-pc-windows-msvc`.
   - `.\target\debug\croft.exe --version` and `--build-info` print valid version, git commit hash, and UTC ISO-8601 build time.
   - `.\target\debug\croft.exe attach`, `remote foo`, `web`, `view foo` exit non-zero with clear Windows unsupported messages and zero panics.
3. **Interactive smoke check (Windows Terminal + WezTerm):**
   - Run `.\target\debug\croft.exe .` inside PowerShell in both Windows Terminal and WezTerm.
   - Verify: Explorer tree loads, opening/editing/saving a file works, `Ctrl+C`/`Ctrl+V` clipboard round-trips with Windows apps, `Ctrl+Shift+F` search works, Git status populates, bottom PowerShell ConPTY pane executes commands (`Get-Location`, `echo hello`), and `Ctrl+Q` exits cleanly without hanging.
