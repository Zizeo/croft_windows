# Windows Port Audit Fixes — Design Spec

**Date:** 2026-11-25  
**Branch:** `refractor` (based on `12fa83142`)  
**Scope:** All 16 findings from the `/ponytail-audit` review of the native Windows port (`#1`–`#16`), including retargeting self-update and prebuilt release URLs (`#16`) to `Zizeo/croft_windows`.

---

## Intent & Why

The native Windows port (`x86_64-pc-windows-msvc`) now builds and passes CI, but a full branch audit found 16 defects spanning command injection, undefined behavior, runtime panics on non-ASCII process names, UI-thread IPC stalls, leaked SSH tunnel processes, broken PowerShell/Cmd quoting, inconsistent executable detection, and stale upstream URLs/docs. Fixing all 16 in a single risk-ordered pass hardens the Windows port to production quality while keeping the Unix/Linux/macOS implementation untouched.

## Goals, Non-Goals, and Anti-Goals

**Goals:**
- Eliminate all command-injection vectors (`#1`, `#6`) and memory/slice safety bugs (`#2`, `#3`).
- Eliminate Windows IPC weaknesses (`#5`), SSH tunnel leaks / false-positive forwards (`#7`), unbounded ConPTY buffering (`#8`), and unmanaged `pdftoppm` spawns (`#13`).
- Make Windows shell commands, task isolation, and toolchain discovery (`#4`, `#9`, `#10`, `#11`, `#12`, `#15`) work reliably across PowerShell 5.1/7+ and `cmd.exe`.
- Bring `README.md` (`#14`) and self-update/release URLs (`#16`) in line with `Zizeo/croft_windows`.

**Non-Goals:**
- Changing Unix (`#[cfg(unix)]`) behavior, wire formats, or test expectations except where `#16` updates the repository slug for all platforms.
- Porting Unix-only daemon features (`croft attach`, `croft web`, `croft collab`, etc.).
- Introducing new third-party crates.

**Anti-Goals:**
- Adding `unsafe` Win32 FFI outside `src/platform/`.
- Breaking existing synchronous `LocalListener::accept()` tests (`tests/platform.rs`, `src/app/tests/agents.rs`) with thread-scheduling races.
- Breaking `scripts/check_doc_ownership.py` (attributes must stay below `///` doc comments) or `scripts/check_binstall_contract.py`.
- Staging or committing the untracked `ci-failure/` directory.

## Global Constraints

- **No new crates:** Reuse existing dependencies (`windows-sys 0.59` with already-enabled `Win32_UI_Shell`, `ring` for `SystemRandom`, `dunce`).
- **FFI confinement:** All `unsafe` Win32 FFI stays inside `src/platform/`.
- **Doc ownership:** Keep `///` doc comments attached to items (`#[cfg(...)]` / `#[allow(...)]` attributes go below `///`).
- **Verification gates:** Every change must pass `cargo fmt --check`, Windows & Linux (WSL) `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked --test platform`, `cargo test --locked --test cli`, targeted `--bin croft` unit tests, `python3 scripts/check_doc_ownership.py origin/main HEAD`, and `python3 scripts/check_binstall_contract.py`.

---

## Approaches Considered

### Structure
- **Approach A (Chosen):** One unified spec and one implementation plan grouped into 4 risk-first phases (~14–16 atomic TDD tasks, 1–2 files per task). Minimal ceremony because the fixes are largely independent and share only two small helpers (`strip_windows_exe`, `GITHUB_REPO`).
- **Approach B (Rejected):** Three separate sub-project specs and plans (Security/Core, Shells/Tooling, Branding/Docs). Adds 3× spec/plan overhead without meaningful architectural separation.

### Key Internal Design Decisions
1. **Non-blocking Windows IPC auth (`#5`, `src/platform/ipc.rs`) — Revised to non-blocking `PendingAuth` queue (planner call, flagged for review):**
   - *Alternative rejected:* Background acceptor/auth thread feeding an `mpsc::sync_channel`. Inspecting `tests/platform.rs:749-775` showed tests call `listener.set_nonblocking(true)`, connect via `LocalStream::connect()`, and call `listener.accept().unwrap()` immediately on the next line with no retry loop, and also assert `PermissionDenied` on an immediate rogue connection. A background thread introduces scheduling races on those deterministic single-thread tests and requires shutdown wakeups on `Drop`.
   - *Chosen design:* Maintain an internal `pending: Mutex<Vec<PendingAuth>>` queue (capped at 32 peers) inside `LocalListener`. In non-blocking mode, `accept()` drains `self.inner.accept()` until `WouldBlock` (marking each accepted socket non-blocking + nodelay), then advances pending peers non-blockingly. Already-arrived valid tokens succeed in the same `accept()` call (zero race); stalled peers return `WouldBlock` immediately without stalling the UI thread or other valid clients, and expire after `ACCEPT_AUTH_TIMEOUT` (100 ms).
2. **Windows SSH forward readiness (`#7`, `src/remote/forward.rs`):**
   - *Alternative rejected:* Active `TcpStream::connect_timeout` probe to `127.0.0.1:local_port` (triggers a spurious SSH channel open to the remote service).
   - *Chosen design:* Poll `managed.child_mut().try_wait()` together with `crate::platform::net::list_tcp_listeners()` checking for `(port == local_port, pid == ssh_pid)` up to a 5 s deadline (50 ms ticks), behind a pure/injectable helper so unit tests exercise fast bind, slow bind (>150 ms), early exit, and timeout without spawning real `ssh`.

---

## Detailed Design by Phase

### Phase 1 — Security, UB & Panics (`#1`, `#2`, `#3`, `#6`)

#### `#1` Safe URL & target opening via `ShellExecuteW` (`src/platform/os.rs`, `src/remote/forward.rs`, `src/app/mod.rs`)
- In `src/platform/os.rs`, implement shared Windows `shell_execute_open(target: &str) -> std::io::Result<()>` (`#[cfg(windows)]`) and expose two entrypoints:
  - `pub fn open_url(url: &str) -> std::io::Result<()>` (for background relay `src/remote/forward.rs`): rejects `\0`/`\r`/`\n`/`\t` (`InvalidInput`), runs `shell_execute_open(url)` on Windows, and runs `xdg-open` / `open` with `.status()` (requiring `status.success()`) on `#[cfg(not(windows))]`.
  - `pub fn spawn_open_target(target: &str) -> std::io::Result<()>` (for UI-thread `src/app/mod.rs::open_url`, which handles both URLs and local media paths from `src/app/commands.rs:1680`): rejects `\0`/`\r`/`\n`/`\t`, runs `shell_execute_open(target)` on Windows (returns immediately after shell handoff while still surfacing immediate `ShellExecuteW` launch errors like `FileNotFound`), and spawns `xdg-open` / `open` via `.spawn()` with `Stdio::null()` on `#[cfg(not(windows))]` so the frame loop never blocks.
- `shell_execute_open` encodes `"open\0"` and `target\0` to UTF-16 and calls `windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)` (`const SW_SHOWNORMAL: i32 = 1`; `Win32_UI_Shell` is already enabled in `Cargo.toml:304`). Return value `(rc as isize) > 32` maps to `Ok(())`, else `Err(io::Error::other(...))`.
- Update `src/remote/forward.rs::open_local_url` to call `crate::platform::os::open_url(url)` (gated by `url_is_safe_to_open(url)`), and update `src/app/mod.rs::open_url` to call `crate::platform::os::spawn_open_target(url)`, eliminating both `cmd /C start` command-injection sites while preserving `&`, `|`, `^`, `%`, `"` and local media paths.

#### `#2` Byte-safe `.exe` suffix stripping (`src/platform/process.rs`, `src/widgets/terminal/processes.rs`, `src/port_detect.rs`)
- Add `pub fn strip_windows_exe(name: &str) -> &str` in `src/platform/process.rs`:
  - Check `name.len() > 4 && name.as_bytes()[name.len() - 4..].eq_ignore_ascii_case(b".exe")`. Because `b".exe"` is 4 ASCII bytes, matching it guarantees `name.len() - 4` is a UTF-8 char boundary.
  - Return `&name[..name.len() - 4]` if matched, else `name`.
- Replace the duplicated slicing logic in `src/widgets/terminal/processes.rs::strip_windows_exe` (delegate or import) and `src/port_detect.rs::windows_subtree_and_names` with `crate::platform::process::strip_windows_exe`.
- Unit tests cover ASCII (`node.exe`, `NODE.EXE`, `.exe`, `cmd`) and multi-byte non-ASCII names (`"日本語"`, `"日本語.exe"`, `"é.e"`, `"🦀"`).

#### `#3` Aligned buffer for `GetExtendedTcpTable` (`src/platform/net.rs`)
- Change `fetch_tcp_table(family: u32) -> Option<(Vec<u32>, usize)>` to allocate `Vec<u32>` with `(byte_capacity + 3) / 4` elements and pass `buf.as_mut_ptr().cast::<c_void>()` with `buf_len = (buf.len() * 4) as u32`. On `NO_ERROR`, return `Some((buf, buf_len as usize))`.
- Add compile-time assertions verifying `align_of::<MIB_TCPTABLE_OWNER_PID>() <= align_of::<u32>()` and `align_of::<MIB_TCP6TABLE_OWNER_PID>() <= align_of::<u32>()`.
- Update `query_tcp4_listeners` and `query_tcp6_listeners` to cast `buf.as_ptr().cast::<MIB_TCP*TABLE_OWNER_PID>()` and bounds-check against `byte_len`.

#### `#6` Direct spawn for Windows "New Window" fallback (`src/platform/process.rs`, `src/app/editor_tabs.rs`)
- Add `pub fn configure_new_console(cmd: &mut Command)` (gated `#[cfg(windows)]`) in `src/platform/process.rs` setting `CommandExt::creation_flags(CREATE_NEW_CONSOLE)` (`windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE`).
- In `src/app/editor_tabs.rs::windows_new_window_command`: for `WindowsHostTerminal::Fallback`, return `(exe, vec![root, "--open-file", file, "--zen"])` directly instead of `cmd.exe /C start "" ...`.
- In `spawn_new_window`: when `host == WindowsHostTerminal::Fallback`, call `crate::platform::process::configure_new_console(&mut cmd)` before `.spawn()`.
- Update existing `editor_tabs` unit tests and add a test with `&` and `%TEMP%` in `root`/`file` proving `cmd.exe` is never invoked.

---

### Phase 2 — IPC, Process Lifecycle & Backpressure (`#5`, `#7`, `#8`, `#13`)

#### `#5` Hardened Windows `LocalListener` (`src/platform/ipc.rs`)
- **CSPRNG nonce:** Replace `RandomState` hashing with `ring::rand::SystemRandom::new().fill(&mut bytes)` (`[u8; 16]`), hex-encoded to 32 chars, returning `io::Result<String>`.
- **Non-blocking `PendingAuth` queue:**
  - Store `pending: Mutex<Vec<PendingAuth>>` on `LocalListener`, where `PendingAuth { stream: TcpStream, buf: Vec<u8>, deadline: Instant }`. Cap queue at `MAX_PENDING_AUTH = 32` (drop oldest expired or reject excess with immediate close).
  - In `accept()` when `self.nonblocking.load(Ordering::Relaxed)` is `true`:
    1. Drain `self.inner.accept()` until `WouldBlock`, calling `set_nodelay(true)` and `set_nonblocking(true)` on each stream and pushing to `pending`.
    2. Iterate `pending` (retaining incomplete peers): read bytes non-blockingly up to `\n` or `MAX_TOKEN_LINE`.
       - Complete matching token -> remove peer, return `Ok((LocalStream { inner }, ()))`.
       - Complete mismatch / oversized token / non-empty EOF / expired deadline -> remove peer, return `Err(PermissionDenied)`.
       - Empty EOF (`Ok(0)` with `buf.is_empty()`) -> remove peer (liveness probe), continue scanning.
       - `WouldBlock` -> keep peer in `pending`, continue scanning next peer.
    3. If no peer completed, return `Err(io::Error::from(io::ErrorKind::WouldBlock))`.
  - In `accept()` when `nonblocking == false`: keep bounded blocking auth (`ACCEPT_AUTH_TIMEOUT = 100 ms`).
- **Stale descriptor PID check:**
  - Write `<port>\n<nonce>\n<pid>\n` in `bind()`.
  - Update `parse_descriptor` to return `io::Result<(u16, String, Option<u32>)>` (3rd line optional for backward compatibility).
  - On `bind()`, if an existing descriptor parses:
    - If `Some(pid)` and `!crate::platform::process::is_pid_alive(pid)` -> stale, skip TCP probe and overwrite.
    - Otherwise (live PID or legacy 2-line file) -> run existing `TcpStream::connect_timeout` probe and return `AddrInUse` if it connects.
- Tests in `tests/platform.rs`:
  - Stalled rogue peer connecting without sending bytes does NOT block `accept()` (returns `WouldBlock` in < 25 ms) AND does NOT block a subsequent valid `LocalStream::connect()` from succeeding immediately.
  - Descriptor with a dead PID + live unrelated TCP listener on that port is treated as stale and successfully overwritten by `LocalListener::bind()`.

#### `#7` Managed Windows SSH forwards with real readiness check (`src/remote/forward.rs`)
- Change `WINDOWS_FORWARDS` from `BTreeMap<(u16, u16), Child>` to `BTreeMap<(u16, u16), crate::platform::process::ManagedChild>`.
- Spawn `ssh -N -T -o BatchMode=yes -o ExitOnForwardFailure=yes -L <spec> <host>` via `ManagedChild::spawn_in_group(&mut cmd)` (Job Object auto-kills on `croft` exit; `CREATE_NO_WINDOW`).
- Extract a testable helper `wait_for_forward_ready(timeout, poll_interval, check_exited, check_listening) -> bool` (timeout = 5 s, poll = 50 ms):
  - Returns `false` immediately if `check_exited()` is true.
  - Returns `true` as soon as `check_listening()` reports `(port == local_port, pid == ssh_pid)` via `crate::platform::net::list_tcp_listeners()`.
  - Returns `false` on timeout.
- If not ready: call `managed.kill_tree()` and return `false`.
- If ready: insert into `WINDOWS_FORWARDS` (calling `old.kill_tree()` if an older tunnel occupied `(local_port, remote_port)`).
- On `handle_unforward_request`: remove from `WINDOWS_FORWARDS` and call `managed.kill_tree()`.

#### `#8` Bounded ConPTY output channel (`src/widgets/terminal/backend.rs`)
- Replace `std::sync::mpsc::channel::<Vec<u8>>()` in `PaneLink::local` (line 308) with `std::sync::mpsc::sync_channel::<Vec<u8>>(16)` (~1 MiB buffer cap at 64 KiB chunks).
- Note: `Receiver<Vec<u8>>` type is identical for both `channel` and `sync_channel`, so no field or caller signature changes are needed.

#### `#13` Managed Windows `pdftoppm` with PATH resolution (`src/pdf.rs`)
- In `rasterize_with_pdftoppm`: resolve `let program = crate::platform::env::resolve_on_path("pdftoppm").unwrap_or_else(|| PathBuf::from("pdftoppm"));` and pass `&program` to `run_pdftoppm`.
- In `run_pdftoppm`:
  - Keep `#[cfg(unix)]` unchanged.
  - On `#[cfg(not(unix))]`: spawn via `crate::platform::process::ManagedChild::spawn_in_group(&mut cmd)?`, split into `(mut child, killer)` via `split_killer()`, and pass `killer` to `kill_group_managed(child, killer, guard)` which calls `killer.kill_tree()` before spawning the reaping/cleanup thread.

---

### Phase 3 — Shells, Installers & Executable Discovery (`#4`, `#9`, `#10`, `#11`, `#12`, `#15`)

#### `#4` Shell-safe Windows Go Delve install command (`src/dap/install.rs`, `src/app/debugger/adapters.rs`)
- Update `DLV_INSTALL_WINDOWS` to single-quoted PowerShell syntax so outer PowerShell does not expand `$v`, `$env:GOBIN`, or `$HOME`:
  ```text
  powershell -NoProfile -Command '$v = if ((go env GOVERSION) -match ''^go1\.([0-9]($|\.)|1[0-9]|2[0-4])'') { ''v1.25.2'' } else { ''latest'' }; $env:GOBIN = Join-Path $HOME ''.croft\servers\go''; go install github.com/go-delve/delve/cmd/dlv@$v'
  ```
- Keep `DLV_INSTALL_WINDOWS_CMD` (double-quoted) and add `pub fn dlv_install_command(flavor: crate::tasks::ShellFlavor) -> &'static str` mapping `Posix -> DLV_INSTALL_UNIX`, `PowerShell -> DLV_INSTALL_WINDOWS`, `Cmd -> DLV_INSTALL_WINDOWS_CMD`.
- Wire `src/app/debugger/adapters.rs::install_delve` to select via `dlv_install_command(term.shell_flavor())` (or `ShellFlavor::for_shell_path` matching how `PtyTerminal` determines its flavor).
- Add a `#[cfg(windows)]` unit test running `DLV_INSTALL_WINDOWS` through an outer `powershell.exe -NoProfile -Command` with a stub `go.cmd` for both `go1.24.6` (`@v1.25.2`) and `go1.25.0` (`@latest`).

#### `#9` & `#15` Safe `cd` commands and isolated task `cwd`/`env` (`src/widgets/terminal/processes.rs`, `src/tasks.rs`)
- **`format_cd_command_for` (`src/widgets/terminal/processes.rs`):**
  - Strip `\r`, `\n`, and `\0` from `path` for `PowerShell` and `Cmd` (and ensure no control-char injection into the shell line).
  - `ShellFlavor::PowerShell`: emit `\x1bSet-Location -LiteralPath '<single-quoted>'\r` (doubling `'`).
  - `ShellFlavor::Cmd`: emit `\x1bcd /d "<escaped>"\r`, stripping `"`, `\r`, `\n`, `\0` and escaping `%` as `"^%"` so `%VAR%` is not expanded by interactive `cmd.exe`.
- **Task runner (`src/tasks.rs::command_line_for`):**
  - `ShellFlavor::PowerShell`:
    - Use `Push-Location -LiteralPath {quoted_cwd}` when `cwd` is set.
    - For each `(i, (key, value))` in `v.env`, save the prior value (`$__croft_env_{i} = [System.Environment]::GetEnvironmentVariable({quoted_key}, 'Process')`), set via `[System.Environment]::SetEnvironmentVariable(...)` or `Set-Item` / `$env:{key}`, and restore (or delete when prior was `$null`) inside the `finally { ... }` block alongside `Pop-Location`. Wrap with `try { {line} } finally { ... }` even when only `env` (without `cwd`) is set.
  - `ShellFlavor::Cmd` (planner call, flagged for review):
    - When `cwd` and/or `env` is set, isolate them from the interactive `cmd.exe` pane by running them inside `cmd /d /c <escaped>` **without** an outer quote wrapper around the whole command (instead joining `cd /d <cwd>`, `set "K=V"`, and `line` into `inner = format!("{} && {line}", setup.join(" && "))`, then applying `escape_unquoted_cmd_meta(&inner)` which tracks `in_quotes: bool` across `"` and caret-escapes `^`, `&`, `|`, `<`, `>`, `(`, `)` only when `!in_quotes`). Because the first token after `cmd /d /c ` is always `cd` or `set` (never `"`), `cmd.exe`'s `/c` quote-stripping never triggers, while balanced quotes protect embedded metacharacters (such as `&` inside `set "K=a&b"`) and caret-escaped unquoted operators survive the outer `cmd.exe` pass intact.
  - Add Windows integration tests running generated PowerShell and Cmd task lines against real `powershell.exe` and `cmd.exe` verifying `cwd` (including `[brackets]`) and `env` do NOT leak after the task exits.

#### `#10` Non-interactive Windows `zoxide` installer (`src/zoxide.rs`)
- Update `WINDOWS_INSTALL_SCRIPT` to:
  `"winget install --id ajeetdsouza.zoxide -e --accept-package-agreements --accept-source-agreements"`.
- In `install_in_background()` under `#[cfg(windows)]`: spawn `Command::new("winget")` directly with args `["install", "--id", "ajeetdsouza.zoxide", "-e", "--accept-package-agreements", "--accept-source-agreements"]` and `Stdio::null()` stdin (plus `detach_from_tty`), instead of `cmd.exe /C`.
- Report `"could not start winget: {e}"` on `#[cfg(windows)]` and `"could not start sh: {e}"` on `#[cfg(not(windows))]`.

#### `#11` Windows off-PATH Node discovery & fixed probe (`src/lsp/install.rs`)
- Extend `well_known_node_dirs()` on Windows with:
  - `%VOLTA_HOME%\bin`
  - `%SCOOP%\apps\nodejs\current`, `%SCOOP%\apps\nodejs-lts\current`, and `%USERPROFILE%\scoop\apps\nodejs\current` / `nodejs-lts\current`
- Add `fnm_node_dir()` on Windows scanning `%FNM_DIR%\node-versions\<ver>\installation` (or `%APPDATA%\fnm\node-versions\<ver>\installation` / `%LOCALAPPDATA%\fnm_multishells`) picking the highest semver dir with `node` + `npm`.
- Fix `node_probe_command()` on Windows: replace broken `cmd.exe /d /s /c node -e <script>` with PowerShell (`pwsh` if resolved on PATH, else `powershell.exe`) running `-Command` with a single-quoted command invoking `node -e 'process.stdout.write("__CROFT_NODE__"+process.execPath)'` (no `-NoProfile` so profile-based `fnm`/custom PATH setup runs, detached via `detach_from_tty` and bounded by the existing 15 s timeout).

#### `#12` Unified Windows executable detection (`src/platform/env.rs`, `src/dap/install.rs`, `src/lsp/install.rs`)
- Delegate `crate::platform::env::is_executable_file(path)` directly to `crate::platform::fs::is_executable(path)` (single source of truth: `0o111` on Unix, `PATHEXT` on Windows).
- In `executable_in_dir(dir, cmd)` on Windows:
  - If `cmd` has an extension, check `is_executable_file(&dir.join(cmd))`.
  - Otherwise probe `[".exe", ".cmd", ".bat"]` (and drop the bare `""` fallback so extensionless POSIX `#!/bin/sh` shims are never returned on Windows).
- In `find_dlv_in`: restrict the bare `d.join("dlv").is_file()` fallback to `#[cfg(unix)]` (on Windows only `executable_in_dir(&d, "dlv")` matches).
- Update Windows-unaware test fixtures that create extensionless files:
  - `src/dap/install.rs::find_dlv_searches_path_then_gobin_then_gopath_then_croft` -> use `dlv.exe` on Windows, `dlv` on Unix.
  - `src/lsp/install.rs::nvm_node_dir_picks_highest_version_with_a_node_binary` -> use `node.exe` on Windows, `node` on Unix (and `chmod 0o755` on Unix if needed; verify existing Unix behavior).
- Add a Windows test in `tests/platform.rs` asserting `executable_in_dir` returns `None` when a directory contains ONLY a bare extensionless POSIX shim `vtsls`.

---

### Phase 4 — Fork Branding & Docs (`#14`, `#16`)

#### `#14` `README.md` & `Win32_UI_Shell` (`README.md`, `Cargo.toml`)
- Keep `Win32_UI_Shell` in `Cargo.toml:304` (now actively used by `ShellExecuteW` in `#1`).
- Update `README.md:68-70` to state accurately that Windows CI runs `cargo fmt`, `cargo clippy`, `tests/platform`, `tests/cli`, and `cargo test --bin croft` (with Unix-only daemon tests gated).

#### `#16` Fork self-update & release URLs (`src/update_check.rs`, `src/remote_prebuilt.rs`, `Cargo.toml`, tests)
- Define `pub const GITHUB_REPO: &str = "Zizeo/croft_windows";` in `src/update_check.rs` and derive:
  - `RELEASES_LATEST_URL` = `"https://api.github.com/repos/Zizeo/croft_windows/releases/latest"`
  - `release_notes_url(version)` = `format!("https://github.com/{GITHUB_REPO}/releases/tag/v{version}")`
  - `remote_prebuilt::RELEASES` = `"https://github.com/Zizeo/croft_windows/releases/download"` (derived or `concat!` from `GITHUB_REPO`)
  - `remote_prebuilt::release_identity(version)` = `format!("https://github.com/{GITHUB_REPO}/.github/workflows/release.yml@refs/tags/v{version}")`
- Update `Cargo.toml:7` `repository = "https://github.com/Zizeo/croft_windows"`.
- Update tests asserting the release URLs (`src/app/tests/updates.rs:509`, `src/remote_prebuilt.rs:587`, `src/update_check.rs:1234`), leaving historical issue references (`vitali87/croft#...`) and unrelated mock fixtures untouched.
- Verify `python3 scripts/check_binstall_contract.py` still passes.

---

## What's Left to the Builder (Implementation Plan)

- Exact test helper naming, temp-dir layout, and commit granularity (one commit per task or logical pair such as `#9`+`#15`).
- Exact helper visibility (`pub` vs `pub(crate)`) where internal to the binary crate.
- Exact ordering of assertions inside each unit/integration test.
