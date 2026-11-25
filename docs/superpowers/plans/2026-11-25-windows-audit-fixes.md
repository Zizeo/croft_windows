# Windows Port Audit Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all 16 Windows-port audit findings (`#1`–`#16`) in four risk-ordered phases while keeping Unix/Linux/macOS behavior intact and retargeting self-update/release URLs to `Zizeo/croft_windows`.

**Architecture:** Confine all `unsafe` Win32 FFI (`ShellExecuteW`, aligned `GetExtendedTcpTable`, `CREATE_NEW_CONSOLE`, Job Objects) inside `src/platform/`, fix non-blocking Windows IPC and ConPTY backpressure without breaking deterministic single-threaded tests, harden PowerShell/Cmd command generation and toolchain discovery, and centralize fork repository URLs behind a single constant.

**Tech Stack:** Rust 1.97+ (edition 2024), `windows-sys` 0.59 (`Win32_UI_Shell`, `Win32_System_Threading`, `Win32_System_JobObjects`, `Win32_NetworkManagement_IpHelper`), `ring::rand::SystemRandom`, `dunce`, PowerShell / `cmd.exe`, WSL2 Linux toolchain for cross-platform clippy/doc-ownership verification.

**Spec:** [docs/superpowers/specs/2026-11-25-windows-audit-fixes-design.md](../specs/2026-11-25-windows-audit-fixes-design.md)

## Global Constraints

- **No new crates:** Reuse existing dependencies (`windows-sys 0.59` with already-enabled `Win32_UI_Shell`, `ring` for `SystemRandom`, `dunce`).
- **FFI confinement:** All `unsafe` Win32 FFI stays inside `src/platform/`.
- **Doc ownership:** Keep `///` doc comments attached to items (`#[cfg(...)]` / `#[allow(...)]` attributes go below `///`).
- **Untracked hygiene:** Never stage or commit `ci-failure/`.
- **WSL target dir:** When running `cargo` inside WSL, always set `CARGO_TARGET_DIR=/home/elio/.cache/croft-linux-target` (literal path, not `$HOME`) because `/tmp` tmpfs is small.
- **Verification gates:** Final branch must pass `cargo fmt --check`, Windows & Linux (WSL) `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked --test platform`, `cargo test --locked --test cli`, targeted `--bin croft` unit tests, `python3 scripts/check_doc_ownership.py origin/main HEAD`, and `python3 scripts/check_binstall_contract.py`.

## Review Focus

1. **Direct control-char / NUL injection into `platform::os::open_url` / `spawn_open_target` (Task 1):** A caller bypassing `forward.rs`'s validator passes a URL or local path containing `\0`, `\r`, `\n`, or `\t` — must fail with `io::ErrorKind::InvalidInput` before reaching `ShellExecuteW` or `open`/`xdg-open`.
2. **Legacy 2-line IPC descriptor vs. dead-PID 3-line descriptor (Task 5):** A 2-line descriptor (`<port>\n<nonce>\n`) left by an older build with a live listener must still fall back to the TCP probe and return `AddrInUse`, whereas a 3-line descriptor whose PID is dead must be treated as stale and overwritten even if another process recycled that port.
3. **Pending-auth connection flood on non-blocking Windows IPC (Task 5):** More than `MAX_PENDING_AUTH` (32) stalled loopback clients connecting without sending a token must be bounded/evicted without unbounded memory growth and must not starve a valid `LocalStream::connect` client.
4. **PowerShell task env restoration for newly created vs. pre-existing vars on failure (Task 10):** A PowerShell task that sets one pre-existing env var and one previously unset env var — and then fails — must restore the original value for the first AND completely remove the second (`Test-Path Env:... == False`), rather than leaving an empty string.
5. **Explicit non-executable extension in `executable_in_dir` (Task 13):** Calling `executable_in_dir(dir, "README.txt")` on Windows must return `None` (not `Some(...)`), while `executable_in_dir(dir, "tool.exe")` returns `Some(...)` via `PATHEXT`.

---

## File Structure

| File | Responsibility in This Plan |
|---|---|
| `src/platform/os.rs` | Add `open_url` and `spawn_open_target` backed by Win32 `ShellExecuteW` (`#1`). |
| `src/remote/forward.rs` | Delegate `open_local_url` to `platform::os::open_url` (`#1`); switch `WINDOWS_FORWARDS` to `ManagedChild` with TCP-listener readiness poll (`#7`). |
| `src/app/mod.rs` | Delegate UI-thread `open_url` to `platform::os::spawn_open_target` (`#1`). |
| `src/platform/process.rs` | Add shared byte-safe `strip_windows_exe` (`#2`) and `configure_new_console` (`#6`). |
| `src/widgets/terminal/processes.rs` | Reuse `platform::process::strip_windows_exe` (`#2`); harden `format_cd_command_for` (`-LiteralPath`, `\r`/`\n`/`\0`, `%`) (`#9`, `#15`). |
| `src/port_detect.rs` | Reuse `platform::process::strip_windows_exe` (`#2`). |
| `src/platform/net.rs` | Allocate 4-byte-aligned `Vec<u32>` for `GetExtendedTcpTable` (`#3`). |
| `src/app/editor_tabs.rs` | Direct `exe` + `CREATE_NEW_CONSOLE` spawn for Windows fallback (`#6`). |
| `src/platform/ipc.rs` | `ring::rand::SystemRandom` nonce, non-blocking `PendingAuth` queue, 3-line PID-checked descriptor (`#5`). |
| `src/widgets/terminal/backend.rs` | Bounded `sync_channel(16)` for ConPTY output (`#8`). |
| `src/pdf.rs` | Resolve `pdftoppm` via `resolve_on_path` and spawn via `ManagedChild::spawn_in_group` on Windows (`#13`). |
| `src/dap/install.rs` | Single-quoted `DLV_INSTALL_WINDOWS`, `dlv_install_command(ShellFlavor)`, Windows test (`#4`); Windows `.exe` fixture update (`#12`). |
| `src/app/debugger/adapters.rs` | Use `dlv_install_command(ShellFlavor::host_default())` (`#4`). |
| `src/tasks.rs` | PowerShell `-LiteralPath` + env save/restore in `finally`; Cmd `cmd /d /c <escape_unquoted_cmd_meta>` isolation (`#9`, `#15`). |
| `src/zoxide.rs` | Direct `winget` spawn with `--accept-*-agreements` and accurate error string (`#10`). |
| `src/lsp/install.rs` | `VOLTA_HOME` / `FNM_DIR` / Scoop discovery + PowerShell `node_probe_command` (`#11`); Windows `.exe` fixture update (`#12`). |
| `src/platform/env.rs` | Delegate `is_executable_file` to `fs::is_executable`, drop bare `""` Windows probe (`#12`). |
| `README.md` | Update Windows test/CI status (`#14`). |
| `src/update_check.rs`, `src/remote_prebuilt.rs`, `Cargo.toml` | Centralize `GITHUB_REPO = "Zizeo/croft_windows"` (`#16`). |
| `tests/platform.rs`, `src/app/tests/tabs.rs`, `src/app/tests/updates.rs`, `src/widgets/terminal/tests.rs` | Unit & integration tests for every task. |

---

## Phase 1 — Security, UB & Panics

### Task 1: Safe URL & target opening via `ShellExecuteW` (`#1`)

**Files:**
- Modify: `src/platform/os.rs:1-60`
- Modify: `src/remote/forward.rs:26-103, 212-215`
- Modify: `src/app/mod.rs:3989-4022`
- Test: `tests/platform.rs`

**Interfaces:**
- Consumes: `windows_sys::Win32::UI::Shell::ShellExecuteW` (already enabled via `Win32_UI_Shell` in `Cargo.toml:304`)
- Produces:
  - `pub fn crate::platform::os::open_url(url: &str) -> std::io::Result<()>`
  - `pub fn crate::platform::os::spawn_open_target(target: &str) -> std::io::Result<()>`

- [ ] **Step 1: Write failing tests in `tests/platform.rs`**

Add tests pinning control-char/NUL rejection (Review Focus #1) and UTF-16 validation on both `open_url` and `spawn_open_target`:

```rust
#[test]
fn os_open_url_and_spawn_open_target_reject_control_chars_and_nul() {
    for bad in [
        "https://example.com/\0evil",
        "https://example.com/\r\ncalc",
        "https://example.com/\tfoo",
        "",
    ] {
        assert_eq!(
            platform::os::open_url(bad).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "open_url must reject {bad:?}"
        );
        assert_eq!(
            platform::os::spawn_open_target(bad).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "spawn_open_target must reject {bad:?}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --test platform os_open_url_and_spawn_open_target_reject_control_chars_and_nul`
Expected: FAIL to compile (`cannot find function open_url in module platform::os`).

- [ ] **Step 3: Implement `open_url` and `spawn_open_target` in `src/platform/os.rs`**

- Add `fn validate_open_target(target: &str) -> std::io::Result<()>` rejecting empty strings and any char in `['\0', '\r', '\n', '\t']` with `io::ErrorKind::InvalidInput`.
- Under `#[cfg(windows)]`, implement `fn shell_execute_open(target: &str) -> std::io::Result<()>` encoding `"open\0"` and `target\0` to `Vec<u16>` and calling `windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)` with `const SW_SHOWNORMAL: i32 = 1;`. Return `Ok(())` if `(rc as isize) > 32`, else `Err(io::Error::other(format!("ShellExecuteW failed with code {}", rc as isize)))`.
- Implement `pub fn open_url(url: &str) -> std::io::Result<()>`: validates target, calls `shell_execute_open(url)` on Windows, or runs `xdg-open` (Linux) / `open` (other Unix) with `Stdio::null()` and `.status()`, returning `Err(io::Error::other(format!("local open(1) exited {status}")))` when `!status.success()`.
- Implement `pub fn spawn_open_target(target: &str) -> std::io::Result<()>`: validates target, calls `shell_execute_open(target)` on Windows, or runs `xdg-open` (Linux) / `open` (other Unix) with `Stdio::null()` and `.spawn()` (non-blocking).

- [ ] **Step 4: Wire `src/remote/forward.rs` and `src/app/mod.rs`**

- In `src/remote/forward.rs:28-52`: change `pub(super) fn open_local_url(url: &str) -> std::io::Result<()>` to delegate directly to `crate::platform::os::open_url(url)`. Update `handle_open_request` (`lines 80-102`) to match `Ok(())` and `Err(e)` (`format!("local open failed: {e}")`).
- In `src/app/mod.rs:3989-4022`: replace the entire body of `fn open_url(url: &str) -> Result<()>` with `crate::platform::os::spawn_open_target(url).with_context(|| format!("opening {url}"))`.

- [ ] **Step 5: Verify tests & clippy**

Run: `cargo test --locked --test platform os_open_url_and_spawn_open_target_reject_control_chars_and_nul && cargo test --locked --bin croft remote::tests::`
Expected: PASS (0 failures).

- [ ] **Step 6: Commit**

```bash
git add src/platform/os.rs src/remote/forward.rs src/app/mod.rs tests/platform.rs
git commit -m "fix(windows): replace cmd.exe URL opening with ShellExecuteW"
```

---

### Task 2: Byte-safe `.exe` suffix stripping (`#2`)

**Files:**
- Modify: `src/platform/process.rs`
- Modify: `src/widgets/terminal/processes.rs:6-14`
- Modify: `src/port_detect.rs:495-501`
- Test: `tests/platform.rs`

**Interfaces:**
- Produces: `pub fn crate::platform::process::strip_windows_exe(name: &str) -> &str`

- [ ] **Step 1: Write failing test in `tests/platform.rs`**

```rust
#[test]
fn process_strip_windows_exe_handles_ascii_and_multibyte_utf8() {
    assert_eq!(platform::process::strip_windows_exe("node.exe"), "node");
    assert_eq!(platform::process::strip_windows_exe("NODE.EXE"), "NODE");
    assert_eq!(platform::process::strip_windows_exe(".exe"), ".exe");
    assert_eq!(platform::process::strip_windows_exe("cmd"), "cmd");
    assert_eq!(platform::process::strip_windows_exe("日本語"), "日本語");
    assert_eq!(platform::process::strip_windows_exe("日本語.exe"), "日本語");
    assert_eq!(platform::process::strip_windows_exe("é.e"), "é.e");
    assert_eq!(platform::process::strip_windows_exe("🦀"), "🦀");
    assert_eq!(platform::process::strip_windows_exe("🦀.ExE"), "🦀");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --test platform process_strip_windows_exe_handles_ascii_and_multibyte_utf8`
Expected: FAIL to compile (`cannot find function strip_windows_exe`).

- [ ] **Step 3: Implement `strip_windows_exe` and deduplicate callers**

- In `src/platform/process.rs`: add `pub fn strip_windows_exe(name: &str) -> &str` checking `name.len() > 4 && name.as_bytes()[name.len() - 4..].eq_ignore_ascii_case(b".exe")`, returning `&name[..name.len() - 4]` else `name`.
- In `src/widgets/terminal/processes.rs:6-13`: replace body of `pub(super) fn strip_windows_exe(name: &str) -> &str` with `crate::platform::process::strip_windows_exe(name)` (preserving existing visibility/tests).
- In `src/port_detect.rs:496-500`: replace the inline `raw[raw.len() - 4..]` slice with `let trimmed = crate::platform::process::strip_windows_exe(&raw);`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --test platform process_strip_windows_exe && cargo test --locked --bin croft port_detect`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/platform/process.rs src/widgets/terminal/processes.rs src/port_detect.rs tests/platform.rs
git commit -m "fix(windows): avoid UTF-8 slice panic when stripping .exe process names"
```

---

### Task 3: 4-byte aligned `Vec<u32>` buffer in Windows TCP table reader (`#3`)

**Files:**
- Modify: `src/platform/net.rs:33-174`
- Test: `tests/platform.rs:786-807`

**Interfaces:**
- Preserves: `pub fn crate::platform::net::list_tcp_listeners() -> Option<Vec<TcpListenerEntry>>`

- [ ] **Step 1: Add alignment-sensitive unit test in `src/platform/net.rs`**

Verify `net_list_tcp_listeners_detects_bound_loopback_socket_and_drop` in `tests/platform.rs` also covers IPv6 (`[::1]:0`) alongside IPv4 (`127.0.0.1:0`), and add a unit test in `src/platform/net.rs` asserting `fetch_tcp_table(AF_INET as u32)` returns a 4-byte-aligned pointer `(buf.as_ptr() as usize) % std::mem::align_of::<MIB_TCPTABLE_OWNER_PID>() == 0`.

```rust
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    #[test]
    fn fetch_tcp_table_returns_properly_aligned_buffer() {
        let (v4, v4_bytes) = fetch_tcp_table(AF_INET as u32).expect("IPv4 table");
        assert!(v4_bytes <= v4.len() * std::mem::size_of::<u32>());
        assert_eq!(
            (v4.as_ptr() as usize) % std::mem::align_of::<MIB_TCPTABLE_OWNER_PID>(),
            0
        );
        if let Some((v6, v6_bytes)) = fetch_tcp_table(AF_INET6 as u32) {
            assert!(v6_bytes <= v6.len() * std::mem::size_of::<u32>());
            assert_eq!(
                (v6.as_ptr() as usize) % std::mem::align_of::<MIB_TCP6TABLE_OWNER_PID>(),
                0
            );
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft platform::net::tests::fetch_tcp_table_returns_properly_aligned_buffer`
Expected: FAIL to compile (`fetch_tcp_table` currently returns `Option<Vec<u8>>`, not `Option<(Vec<u32>, usize)>`).

- [ ] **Step 3: Switch `fetch_tcp_table` to `Vec<u32>` in `src/platform/net.rs`**

- Change `fn fetch_tcp_table(family: u32) -> Option<(Vec<u32>, usize)>`: compute `words = ((size as usize).max(64) + 1024).div_ceil(4)`, allocate `vec![0u32; words]`, pass `buf_len = (buf.len() * 4) as u32`, resize to `((buf_len as usize).saturating_add(1024)).div_ceil(4)` on `ERROR_INSUFFICIENT_BUFFER`, and return `Some((buf, buf_len as usize))` on `NO_ERROR`.
- Add `const { assert!(std::mem::align_of::<MIB_TCPTABLE_OWNER_PID>() <= std::mem::align_of::<u32>()); }` in `query_tcp4_listeners` and the analogous assertion for `MIB_TCP6TABLE_OWNER_PID` in `query_tcp6_listeners`.
- Bounds-check `offset.checked_add(needed)? > byte_len` against the returned `byte_len`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft platform::net::tests && cargo test --locked --test platform net_list_tcp_listeners`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/platform/net.rs tests/platform.rs
git commit -m "fix(windows): align GetExtendedTcpTable buffer to u32 to eliminate UB"
```

---

### Task 4: Direct `CREATE_NEW_CONSOLE` spawn for Windows "New Window" fallback (`#6`)

**Files:**
- Modify: `src/platform/process.rs`
- Modify: `src/app/editor_tabs.rs:2167-2200`
- Test: `src/app/tests/tabs.rs:2740-2811`

**Interfaces:**
- Produces: `#[cfg(windows)] pub fn crate::platform::process::configure_new_console(cmd: &mut std::process::Command)`

- [ ] **Step 1: Update and add failing test assertions in `src/app/tests/tabs.rs`**

Update `windows_new_window_targets_wt_wezterm_or_cmd_fallback` (`lines 2796-2810`) so `WindowsHostTerminal::Fallback` expects `(r"C:\bin\croft.exe", [r"C:\repo", "--open-file", r"C:\repo\main.rs", "--zen"])`, and add a second test proving `&` and `%TEMP%` in `root`/`file` never pass through `cmd.exe`:

```rust
#[cfg(windows)]
#[test]
fn windows_new_window_fallback_spawns_croft_directly_without_cmd_exe() {
    use crate::app::editor_tabs::{WindowsHostTerminal, windows_new_window_command};
    use std::path::Path;

    let exe = Path::new(r"C:\bin\croft.exe");
    let root = Path::new(r"C:\work\a&calc\%TEMP%");
    let file = Path::new(r"C:\work\a&calc\%TEMP%\file&1.rs");
    let (prog, args) = windows_new_window_command(WindowsHostTerminal::Fallback, exe, root, file);
    assert_eq!(prog, r"C:\bin\croft.exe");
    assert_eq!(
        args,
        [
            r"C:\work\a&calc\%TEMP%",
            "--open-file",
            r"C:\work\a&calc\%TEMP%\file&1.rs",
            "--zen",
        ]
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft app::tests::tabs::windows_new_window`
Expected: FAIL (`assertion left == right failed: left: "cmd.exe", right: "C:\\bin\\croft.exe"`).

- [ ] **Step 3: Implement `configure_new_console` and direct fallback spawn**

- In `src/platform/process.rs`: add `#[cfg(windows)] pub fn configure_new_console(cmd: &mut Command)` setting `cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE);`.
- In `src/app/editor_tabs.rs:2167-2180`: change `WindowsHostTerminal::Fallback` to return `(exe, vec![root, OsString::from("--open-file"), file, OsString::from("--zen")])`.
- In `src/app/editor_tabs.rs:2193-2200`: build `let mut cmd = std::process::Command::new(program);`, apply `if host == WindowsHostTerminal::Fallback { crate::platform::process::configure_new_console(&mut cmd); }`, then configure null stdio and `.spawn()?`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft app::tests::tabs::windows_new_window`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/platform/process.rs src/app/editor_tabs.rs src/app/tests/tabs.rs
git commit -m "fix(windows): spawn new window fallback directly with CREATE_NEW_CONSOLE"
```

---

## Phase 2 — IPC, Process Lifecycle & Backpressure

### Task 5: Hardened Windows `LocalListener` — CSPRNG nonce, non-blocking `PendingAuth` queue, PID-checked descriptors (`#5`)

**Files:**
- Modify: `src/platform/ipc.rs:14-255`
- Test: `tests/platform.rs:724-785`

**Interfaces:**
- Consumes: `ring::rand::{SecureRandom, SystemRandom}`, `crate::platform::process::is_pid_alive`
- Preserves: `LocalListener::{bind, set_nonblocking, local_addr, accept}` and `LocalStream::connect` signatures identically

- [ ] **Step 1: Write failing tests in `tests/platform.rs`**

Add three `#[cfg(windows)]` tests covering (a) non-blocking stalled peer + concurrent valid client, (b) dead-PID 3-line descriptor vs. live legacy 2-line descriptor (Review Focus #2), and (c) pending-auth flood > 32 connections (Review Focus #3):

```rust
#[cfg(windows)]
#[test]
fn ipc_nonblocking_accept_does_not_stall_on_silent_peer_or_flood() {
    use std::io::{ErrorKind, Write};
    use std::time::{Duration, Instant};

    let tmp = tempfile::tempdir().unwrap();
    let sock_path = tmp.path().join("nonblock-stall.sock");
    let listener = platform::ipc::LocalListener::bind(&sock_path).unwrap();
    listener.set_nonblocking(true).unwrap();

    // Hold open 40 silent rogue connections (> MAX_PENDING_AUTH = 32).
    let addr = listener.local_addr().unwrap();
    let _rogues: Vec<_> = (0..40)
        .map(|_| std::net::TcpStream::connect(addr).unwrap())
        .collect();

    let start = Instant::now();
    let err = listener.accept().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WouldBlock);
    assert!(
        start.elapsed() < Duration::from_millis(40),
        "non-blocking accept stalled for {:?}",
        start.elapsed()
    );

    // A legitimate client connecting behind the stalled peers must still succeed immediately.
    let mut client = platform::ipc::LocalStream::connect(&sock_path).unwrap();
    client.write_all(b"ok\n").unwrap();
    client.flush().unwrap();
    let (_server, _) = listener
        .accept()
        .expect("valid client accepted ahead of stalled peers");
}

#[cfg(windows)]
#[test]
fn ipc_bind_overwrites_dead_pid_descriptor_and_respects_legacy_two_line_descriptor() {
    let tmp = tempfile::tempdir().unwrap();

    // 1. Live unrelated TCP listener on a port recorded with a dead PID must be overwritten.
    let unrelated = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = unrelated.local_addr().unwrap().port();
    let stale_sock = tmp.path().join("stale-dead-pid.sock");
    std::fs::write(&stale_sock, format!("{port}\ndeadbeef\n99999999\n")).unwrap();
    let rebound = platform::ipc::LocalListener::bind(&stale_sock)
        .expect("dead PID descriptor must be treated as stale");
    drop(rebound);

    // 2. Legacy 2-line descriptor (no PID) pointing at a live listener still returns AddrInUse.
    let legacy_sock = tmp.path().join("legacy-two-line.sock");
    std::fs::write(&legacy_sock, format!("{port}\nlegacytoken\n")).unwrap();
    let err = platform::ipc::LocalListener::bind(&legacy_sock).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    drop(unrelated);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --test platform ipc_`
Expected: FAIL (`non-blocking accept stalled` for >= 100 ms and `dead PID descriptor` returns `AddrInUse`).

- [ ] **Step 3: Implement CSPRNG nonce, `PendingAuth` queue, and PID descriptor check in `src/platform/ipc.rs`**

- Replace `generate_nonce()` with `ring::rand::SystemRandom::new().fill(&mut raw)` (`let mut raw = [0u8; 16]`), encoding all 16 bytes as 32 lowercase hex characters and returning `io::Result<String>` (remove unused `RandomState` / `BuildHasher` / `Hasher` imports).
- Update `parse_descriptor(path: &Path) -> io::Result<(u16, String, Option<u32>)>` reading an optional 3rd line parsed as `u32` (filter `> 0`).
- In `LocalListener::bind`:
  - Write `format!("{port}\n{nonce}\n{}\n", std::process::id())`.
  - When `parse_descriptor(&path)` succeeds with `(existing_port, _, existing_pid)`: if `existing_pid.is_some_and(|pid| !super::super::process::is_pid_alive(pid))`, skip TCP probe and overwrite; otherwise run existing `TcpStream::connect_timeout` probe.
- Add `const MAX_PENDING_AUTH: usize = 32;` and `struct PendingAuth { stream: TcpStream, buf: Vec<u8>, deadline: Instant }`, plus `pending: std::sync::Mutex<Vec<PendingAuth>>` on `LocalListener`.
- Implement non-blocking `accept_nonblocking(&self) -> io::Result<(LocalStream, ())>`:
  ```rust
  // 1. Drain newly accepted sockets into `pending` (non-blocking + nodelay).
  //    If `pending.len() >= MAX_PENDING_AUTH`, prune expired entries first;
  //    if still full, drop the oldest pending entry to stay bounded without
  //    blocking new legitimate clients.
  // 2. Walk `pending` from index 0:
  //    - If `Instant::now() >= entry.deadline`, remove and return PermissionDenied.
  //    - Loop reading 1 byte at a time:
  //        Ok(0) if entry.buf.is_empty() => remove (liveness probe), continue next entry
  //        Ok(0) => remove, return PermissionDenied
  //        Ok(_) if byte == b'\n' => remove, verify constant_time_eq -> Ok(LocalStream) or PermissionDenied
  //        Ok(_) if byte != b'\r' => push (or remove & PermissionDenied if len >= MAX_TOKEN_LINE)
  //        Err(WouldBlock) => leave in `pending`, advance index to next entry
  //        Err(Interrupted) => continue read loop
  //        Err(_) => remove, return PermissionDenied
  // 3. Return Err(WouldBlock) if no entry completed.
  ```
- Keep blocking `accept()` path for `self.nonblocking == false` (first draining any leftover `pending` entries if switched from non-blocking to blocking, then blocking `self.inner.accept()` + `authenticate_client`).

- [ ] **Step 4: Run all IPC tests to verify pass**

Run: `cargo test --locked --test platform ipc_ && cargo test --locked --bin croft agent_approval`
Expected: PASS (all existing and new IPC tests pass cleanly).

- [ ] **Step 5: Commit**

```bash
git add src/platform/ipc.rs tests/platform.rs
git commit -m "fix(windows): harden IPC nonce, non-blocking accept, and stale PID check"
```

---

### Task 6: Managed Windows SSH port forwards & real readiness poll (`#7`)

**Files:**
- Modify: `src/remote/forward.rs:120-254`
- Test: `src/remote/forward.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::platform::process::ManagedChild::{spawn_in_group, child_mut, id, kill_tree}`, `crate::platform::net::list_tcp_listeners`
- Produces: `fn wait_for_forward_ready(timeout: Duration, poll: Duration, check_exited: impl FnMut() -> bool, check_listening: impl FnMut() -> bool) -> bool`

- [ ] **Step 1: Write failing unit tests in `src/remote/forward.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_forward_ready_waits_past_150ms_and_handles_exit_and_timeout() {
        let mut ticks = 0u32;
        assert!(wait_for_forward_ready(
            Duration::from_millis(400),
            Duration::from_millis(10),
            || false,
            || {
                ticks += 1;
                ticks >= 4
            },
        ));

        // Early child exit fails immediately even before timeout.
        let mut checks = 0u32;
        assert!(!wait_for_forward_ready(
            Duration::from_secs(5),
            Duration::from_millis(10),
            || {
                checks += 1;
                checks >= 2
            },
            || false,
        ));

        // Timeout returns false when listener never binds.
        assert!(!wait_for_forward_ready(
            Duration::from_millis(30),
            Duration::from_millis(10),
            || false,
            || false,
        ));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft remote::forward::tests::wait_for_forward_ready`
Expected: FAIL to compile (`cannot find function wait_for_forward_ready`).

- [ ] **Step 3: Implement `wait_for_forward_ready` and `ManagedChild` tracking in `src/remote/forward.rs`**

- Change `#[cfg(not(unix))] static WINDOWS_FORWARDS` value type to `crate::platform::process::ManagedChild`.
- Implement `wait_for_forward_ready` polling until `Instant::now() >= deadline`, returning `false` on `check_exited()`, `true` on `check_listening()`, sleeping `poll` between checks.
- In `handle_forward_request` under `#[cfg(not(unix))]`:
  - Spawn `ssh -N -T -o BatchMode=yes -o ExitOnForwardFailure=yes -L <spec> <host>` via `crate::platform::process::ManagedChild::spawn_in_group(&mut cmd)`.
  - Capture `let ssh_pid = managed.id();` and call `wait_for_forward_ready(Duration::from_secs(5), Duration::from_millis(50), || managed.child_mut().try_wait().map_or(true, |s| s.is_some()), || crate::platform::net::list_tcp_listeners().is_some_and(|list| list.iter().any(|e| e.port == local_port && e.pid == ssh_pid)))`.
  - On `true`: insert into `WINDOWS_FORWARDS`, calling `old.kill_tree()` if replacing a previous tunnel.
  - On `false`: call `managed.kill_tree()`.
- In `handle_unforward_request` under `#[cfg(not(unix))]`: remove from `WINDOWS_FORWARDS` and call `managed.kill_tree()`.
- Ensure `wait_for_forward_ready` has `#[cfg_attr(unix, allow(dead_code))]` (below its `///` doc comment!) so Linux clippy stays warning-free.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft remote::forward::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/remote/forward.rs
git commit -m "fix(windows): manage SSH forward tunnels via Job Objects and poll real listener readiness"
```

---

### Task 7: Bounded ConPTY output channel (`#8`)

**Files:**
- Modify: `src/widgets/terminal/backend.rs:308`
- Test: `src/widgets/terminal/backend.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Preserves: `PaneLink::local` signature and `rx: std::sync::mpsc::Receiver<Vec<u8>>`

- [ ] **Step 1: Define bounded constant and failing test in `src/widgets/terminal/backend.rs`**

Expose `const CONPTY_CHANNEL_BOUND: usize = 16;` used by `PaneLink::local` and add a unit test pinning `CONPTY_CHANNEL_BOUND == 16` and verifying `sync_channel(CONPTY_CHANNEL_BOUND)` applies backpressure (`try_send` returns `TrySendError::Full` on item 17).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft widgets::terminal::backend::`
Expected: FAIL to compile (`cannot find value CONPTY_CHANNEL_BOUND`).

- [ ] **Step 3: Switch `PaneLink::local` to `sync_channel(CONPTY_CHANNEL_BOUND)`**

In `src/widgets/terminal/backend.rs:308`: replace `std::sync::mpsc::channel::<Vec<u8>>()` with `std::sync::mpsc::sync_channel::<Vec<u8>>(CONPTY_CHANNEL_BOUND)`.

- [ ] **Step 4: Run terminal tests to verify pass**

Run: `cargo test --locked --bin croft widgets::terminal::backend:: && cargo test --locked --bin croft widgets::terminal::tests::format_cd`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/widgets/terminal/backend.rs
git commit -m "fix(windows): bound ConPTY output channel to prevent unbounded memory growth"
```

---

### Task 8: PATH-resolved, Job-Object-managed Windows `pdftoppm` (`#13`)

**Files:**
- Modify: `src/pdf.rs:665-720, 767-815, 925-942`
- Test: `src/pdf.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `crate::platform::env::resolve_on_path`, `crate::platform::process::{ManagedChild, TreeKiller}`

- [ ] **Step 1: Write failing Windows timeout test in `src/pdf.rs`**

Add a `#[cfg(windows)]` unit test invoking `run_pdftoppm` against a stub `.cmd` script that spawns a child sleep process (`ping -n 10 127.0.0.1 >nul`) with a short `Duration::from_millis(150)` budget, asserting `Err(TimedOut)` is returned promptly (within 1.5 s) and the child tree is terminated.

- [ ] **Step 2: Run test to verify baseline / failure**

Run: `cargo test --locked --bin croft pdf::tests::`
Expected: New test either fails on orphaned child or reveals missing management wrapper.

- [ ] **Step 3: Implement PATH resolution and `ManagedChild` spawning in `src/pdf.rs`**

- In `rasterize_with_pdftoppm`: resolve `let program = crate::platform::env::resolve_on_path("pdftoppm").unwrap_or_else(|| PathBuf::from("pdftoppm"));` and call `run_pdftoppm(&program, pdf, page, PDF_RENDER_BUDGET)`.
- In `run_pdftoppm`:
  - Keep `#[cfg(unix)]` branch unchanged.
  - Under `#[cfg(not(unix))]`: spawn via `let (mut child, killer) = crate::platform::process::ManagedChild::spawn_in_group(&mut cmd)?.split_killer();`, and on timeout or `try_wait` error call `kill_group_managed(child, killer, guard)` which calls `killer.kill_tree()` before spawning the reaping thread that calls `child.kill()`, `child.wait()`, and `drop(scratch)`.

- [ ] **Step 4: Run PDF tests to verify pass**

Run: `cargo test --locked --bin croft pdf::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/pdf.rs
git commit -m "fix(windows): resolve pdftoppm on PATH and manage its process tree via Job Object"
```

---

## Phase 3 — Shells, Installers & Executable Discovery

### Task 9: Shell-safe Windows Go Delve install command (`#4`)

**Files:**
- Modify: `src/dap/install.rs:300-325, 395-492`
- Modify: `src/app/debugger/adapters.rs:84-90`

**Interfaces:**
- Consumes: `crate::tasks::ShellFlavor`
- Produces:
  - `pub const crate::dap::install::DLV_INSTALL_WINDOWS: &str` (single-quoted PowerShell command)
  - `pub const crate::dap::install::DLV_INSTALL_WINDOWS_CMD: &str` (double-quoted Cmd command)
  - `pub fn crate::dap::install::dlv_install_command(flavor: crate::tasks::ShellFlavor) -> &'static str`

- [ ] **Step 1: Write failing Windows test in `src/dap/install.rs`**

Add a `#[cfg(windows)]` test running `DLV_INSTALL_WINDOWS` through an **outer** `powershell.exe -NoProfile -Command <DLV_INSTALL_WINDOWS>` with a stub `go.cmd` on `PATH` for both `go1.24.7` (`dlv@v1.25.2`) and `go1.25.0` (`dlv@latest`), asserting `GOBIN=<home>\.croft\servers\go` and exit success, plus a cross-platform unit test for `dlv_install_command(ShellFlavor::{Posix, PowerShell, Cmd})`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft dap::install::tests::the_delve_install_follows_the_local_go_version`
Expected: FAIL (outer PowerShell expands `$v` and `$env:GOBIN` in current double-quoted `DLV_INSTALL_WINDOWS`).

- [ ] **Step 3: Update `src/dap/install.rs` and `src/app/debugger/adapters.rs`**

- Set `DLV_INSTALL_WINDOWS` to:
  ```rust
  pub const DLV_INSTALL_WINDOWS: &str = "powershell -NoProfile -Command '$v = if ((go env GOVERSION) -match ''^go1\\.([0-9]($|\\.)|1[0-9]|2[0-4])'') { ''v1.25.2'' } else { ''latest'' }; $env:GOBIN = Join-Path $HOME ''.croft\\servers\\go''; go install github.com/go-delve/delve/cmd/dlv@$v'";
  ```
- Add `pub const DLV_INSTALL_WINDOWS_CMD: &str` with the double-quoted form for `cmd.exe`.
- Add `pub fn dlv_install_command(flavor: crate::tasks::ShellFlavor) -> &'static str` matching `Posix => DLV_INSTALL_UNIX`, `PowerShell => DLV_INSTALL_WINDOWS`, `Cmd => DLV_INSTALL_WINDOWS_CMD`.
- In `src/app/debugger/adapters.rs:87`: use `crate::dap::install::dlv_install_command(crate::tasks::ShellFlavor::host_default())`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft dap::install::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/dap/install.rs src/app/debugger/adapters.rs
git commit -m "fix(windows): single-quote DLV_INSTALL_WINDOWS so PowerShell does not pre-expand vars"
```

---

### Task 10: Safe `cd` commands & isolated PowerShell/Cmd task `cwd`/`env` (`#9` & `#15`)

**Files:**
- Modify: `src/widgets/terminal/processes.rs:515-558`
- Modify: `src/widgets/terminal/tests.rs:1089-1109`
- Modify: `src/tasks.rs:368-406, 3149-3225`

**Interfaces:**
- Preserves: `format_cd_command_for(&Path, ShellFlavor) -> Vec<u8>` and `Task::command_line_for(&self, &SubstCtx, ShellFlavor) -> Result<String, String>`
- Adds private helper: `fn escape_unquoted_cmd_meta(s: &str) -> String` in `src/tasks.rs`

- [ ] **Step 1: Update and add failing tests in `src/widgets/terminal/tests.rs` and `src/tasks.rs`**

1. In `src/widgets/terminal/tests.rs:1089-1108`:
   - Update `format_cd_command_for_powershell_uses_esc_and_doubles_apostrophes` to expect `b"\x1bSet-Location -LiteralPath 'C:\\Users\\O''Brien\\My Project'\r"`.
   - Add `format_cd_command_for_strips_newlines_handles_brackets_and_escapes_cmd_percent`:
     - PowerShell with `C:\app\[id]\a\r\nb` -> `\x1bSet-Location -LiteralPath 'C:\app\[id]\ab'\r`.
     - Cmd with `D:\a%TEMP%b\r\nc` -> `\x1bcd /d "D:\a"^%"TEMP"^%"bc"\r`.
2. In `src/tasks.rs:3149-3224`:
   - Update `powershell_flavor_quotes_and_wraps_cwd_and_env` to expect `Push-Location -LiteralPath {expected_cwd}` and env save/restore inside `finally { ... }`.
   - Add `#[cfg(windows)]` integration tests:
     - `powershell_task_isolates_brackets_cwd_and_restores_existing_and_new_env_on_failure` (Review Focus #4): executes the generated PowerShell command line inside `powershell.exe -NoProfile -Command` with a `[brackets]` cwd, one pre-existing env var (`KEEP=orig` -> overridden to `temp`), one unset env var (`TEMP_ONLY=set`), and a failing command (`exit 1` or `throw`), then verifies after the block that `Get-Location` is unchanged, `$env:KEEP == "orig"`, and `Test-Path Env:TEMP_ONLY == False`.
     - `cmd_task_isolates_cwd_and_env_with_metacharacters`: pipes the generated `ShellFlavor::Cmd` command line into `cmd /d /q` stdin with a spaced `cwd`, `env` containing `a&echo BAD`, and `args`, verifying after execution that outer `cd` is unchanged, outer env var is untouched, and `BAD` was never executed in the outer shell.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --bin croft widgets::terminal::tests::format_cd && cargo test --locked --bin croft tasks::tests::powershell_flavor`
Expected: FAIL.

- [ ] **Step 3: Implement `format_cd_command_for` hardening and PowerShell/Cmd task isolation**

- In `src/widgets/terminal/processes.rs:532-556`:
  - `ShellFlavor::PowerShell`: emit `\x1bSet-Location -LiteralPath '`, skip `\r`, `\n`, `\0`, double `'`, finish with `'\r`.
  - `ShellFlavor::Cmd`: emit `\x1bcd /d "`, skip `"`, `\r`, `\n`, `\0`, replace `%` with `"^%"`, finish with `"\r`.
- In `src/tasks.rs:368-405`:
  - `ShellFlavor::PowerShell`:
    - Use `Push-Location -LiteralPath {quoted_cwd}; ` when `cwd` is `Some`.
    - For each `(i, (key, value))` in `v.env.iter().enumerate()`:
      - Quote `key` via `Quoting::Strong.apply_for(key, ShellFlavor::PowerShell)` and `val` via `Quoting::Strong.apply_for(&expand_task_value(value, ctx)?, ShellFlavor::PowerShell)`.
      - Save: `$__croft_env_{i} = [System.Environment]::GetEnvironmentVariable({qkey}, 'Process'); [System.Environment]::SetEnvironmentVariable({qkey}, {val}, 'Process'); `
      - Restore in `finally`: `[System.Environment]::SetEnvironmentVariable({qkey}, $__croft_env_{i}, 'Process')` (passing `$null` deletes the process env var cleanly in .NET/PowerShell!).
    - Always wrap in `& {{ {setup}try {{ {line} }} finally {{ {teardown} }} }}` whenever `cwd.is_some() || !v.env.is_empty()`.
  - `ShellFlavor::Cmd`:
    - Build `setup` with `cd /d <cwd>` and `set "{key}={val}"`, join `inner = format!("{} && {line}", setup.join(" && "))`, and return `Ok(format!("cmd /d /c {}", escape_unquoted_cmd_meta(&inner)))`, where:
      ```rust
      fn escape_unquoted_cmd_meta(s: &str) -> String {
          let mut out = String::with_capacity(s.len() + 16);
          let mut in_quotes = false;
          for c in s.chars() {
              if c == '"' {
                  in_quotes = !in_quotes;
                  out.push(c);
              } else if !in_quotes && matches!(c, '^' | '&' | '|' | '<' | '>' | '(' | ')') {
                  out.push('^');
                  out.push(c);
              } else {
                  out.push(c);
              }
          }
          out
      }
      ```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft widgets::terminal::tests::format_cd && cargo test --locked --bin croft tasks::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/widgets/terminal/processes.rs src/widgets/terminal/tests.rs src/tasks.rs
git commit -m "fix(windows): harden shell cd quoting and isolate PowerShell/Cmd task cwd and env"
```

---

### Task 11: Non-interactive direct `winget` invocation for Windows `zoxide` installer (`#10`)

**Files:**
- Modify: `src/zoxide.rs:52-55, 523-555`
- Test: `src/zoxide.rs` (inline tests)

**Interfaces:**
- Preserves: `pub fn crate::zoxide::install_script(termux: bool) -> &'static str`

- [ ] **Step 1: Write failing tests in `src/zoxide.rs`**

Add assertions verifying `WINDOWS_INSTALL_SCRIPT` contains `--accept-package-agreements` and `--accept-source-agreements`, and add a helper `fn windows_install_command() -> Command` (or `WINDOWS_WINGET_ARGS: &[&str]`) tested to invoke `"winget"` directly (not `"cmd.exe"`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --bin croft zoxide::tests::`
Expected: FAIL.

- [ ] **Step 3: Implement direct `winget` spawn and error label in `src/zoxide.rs`**

- Set `WINDOWS_INSTALL_SCRIPT` to `"winget install --id ajeetdsouza.zoxide -e --accept-package-agreements --accept-source-agreements"`.
- Under `#[cfg(windows)]` in `install_in_background`: spawn `Command::new("winget").args(["install", "--id", "ajeetdsouza.zoxide", "-e", "--accept-package-agreements", "--accept-source-agreements"]).stdin(Stdio::null())` with `crate::platform::process::detach_from_tty(&mut cmd)`.
- Change line 551 error string to `if cfg!(windows) { format!("could not start winget: {e}") } else { format!("could not start sh: {e}") }`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft zoxide::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/zoxide.rs
git commit -m "fix(windows): run winget directly with non-interactive agreement flags for zoxide"
```

---

### Task 12: Windows off-PATH Node discovery & fixed PowerShell probe (`#11`)

**Files:**
- Modify: `src/lsp/install.rs:1615-1696, 1731-1828`
- Test: `src/lsp/install.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Preserves: `pub(crate) fn node_probe_command() -> Command`, `pub(crate) fn parse_node_probe(stdout: &str) -> Option<&str>`
- Adds pure testable helper: `#[cfg(windows)] fn fnm_node_dir_in(versions: &Path) -> Option<PathBuf>`

- [ ] **Step 1: Write failing tests in `src/lsp/install.rs`**

Add `#[cfg(windows)]` tests:
1. `fnm_node_dir_picks_highest_version_with_node_and_npm`: temp dir with `v18.0.0/installation/{node.exe,npm.cmd}` and `v22.5.0/installation/{node.exe,npm.cmd}` picks `v22.5.0/installation`.
2. `windows_node_probe_command_uses_powershell_not_cmd`: asserts `node_probe_command().get_program()` is `pwsh` or `powershell.exe` (never `cmd.exe`), passes `-Command`, and contains `NODE_PROBE_MARKER`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --bin croft lsp::install::tests::fnm_node_dir && cargo test --locked --bin croft lsp::install::tests::windows_node_probe`
Expected: FAIL.

- [ ] **Step 3: Implement `fnm_node_dir`, `VOLTA_HOME`/Scoop dirs, and PowerShell `node_probe_command`**

- In `well_known_node_dirs()` under `#[cfg(windows)]`: also push `%VOLTA_HOME%\bin`, `%SCOOP%\apps\nodejs\current`, `%SCOOP%\apps\nodejs-lts\current`, and `%USERPROFILE%\scoop\apps\nodejs\current` / `nodejs-lts\current`.
- Add `#[cfg(windows)] fn fnm_node_dir() -> Option<PathBuf>` scanning `%FNM_DIR%\node-versions`, `%APPDATA%\fnm\node-versions`, and `%LOCALAPPDATA%\fnm\node-versions` via `fnm_node_dir_in`, wired into `discover_node_dir()` after `nvm_node_dir()`.
- In `node_probe_command()` under `#[cfg(windows)]`: pick `crate::platform::env::resolve_on_path("pwsh").unwrap_or_else(|| PathBuf::from("powershell.exe"))`, pass `["-Command", &format!("node -e 'process.stdout.write(\"{NODE_PROBE_MARKER}\"+process.execPath)'")]` (no `-NoProfile` so profile-based `fnm`/custom PATH loads), set `stdin(Stdio::null())`, `stdout(Stdio::piped())`, `stderr(Stdio::null())`, and call `crate::platform::process::detach_from_tty(&mut cmd)`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --bin croft lsp::install::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lsp/install.rs
git commit -m "fix(windows): add VOLTA_HOME/fnm/Scoop node discovery and fix PowerShell node probe"
```

---

### Task 13: Unified Windows executable detection (`#12`)

**Files:**
- Modify: `src/platform/env.rs:220-275, 339-345`
- Modify: `src/dap/install.rs:495-527`
- Modify: `src/lsp/install.rs:2685-2697`
- Test: `tests/platform.rs:254-288`

**Interfaces:**
- Consumes: `crate::platform::fs::is_executable`
- Preserves: `crate::platform::env::{is_executable_file, executable_in_dir, find_dlv_in}`

- [ ] **Step 1: Write failing tests in `tests/platform.rs`**

Extend `env_executable_in_dir_prefers_windows_extensions_over_posix_shims` (or add a companion `#[cfg(windows)]` test) asserting:
1. A directory containing ONLY a bare POSIX shim `only-shim` (`#!/bin/sh`) returns `None` from `platform::env::executable_in_dir(dir, "only-shim")`.
2. Explicit non-executable extension `README.txt` returns `None` from both `is_executable_file` and `executable_in_dir(dir, "README.txt")`, while `tool.exe` returns `Some(...)` (Review Focus #5).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --test platform env_executable_in_dir`
Expected: FAIL (`executable_in_dir` currently returns `Some(only-shim)` and `Some(README.txt)` on Windows).

- [ ] **Step 3: Delegate `is_executable_file` to `fs::is_executable` and update Windows fixtures**

- In `src/platform/env.rs:222-237`: replace body of `pub fn is_executable_file(path: &Path) -> bool` with `super::fs::is_executable(path)` (update doc comment accurately).
- In `src/platform/env.rs:260`: change `for ext in [".exe", ".cmd", ".bat", ""]` to `for ext in [".exe", ".cmd", ".bat"]` (dropping the bare `""` fallback on Windows).
- In `src/platform/env.rs:340-344`: keep the bare `d.join("dlv").is_file()` fallback only under `#[cfg(unix)]`; on Windows `find_dlv_in` uses `executable_in_dir(&d, "dlv")` only.
- In `src/dap/install.rs:495-527`: use `let dlv_name = if cfg!(windows) { "dlv.exe" } else { "dlv" };` (matching `tests/platform.rs:545`).
- In `src/lsp/install.rs:2685-2697`: use `let node_name = if cfg!(windows) { "node.exe" } else { "node" };` and on `#[cfg(unix)]` ensure `0o755` permissions on the test file (preserving Unix pass).

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --locked --test platform env_ && cargo test --locked --bin croft dap::install::tests::find_dlv && cargo test --locked --bin croft lsp::install::tests::nvm_node_dir`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/platform/env.rs src/dap/install.rs src/lsp/install.rs tests/platform.rs
git commit -m "fix(windows): unify is_executable_file with PATHEXT and ignore bare POSIX shims"
```

---

## Phase 4 — Fork Branding, Docs & Final Verification

### Task 14: Update `README.md` Windows CI & test status (`#14`)

**Files:**
- Modify: `README.md:68-71`
- Check: `Cargo.toml:304` (confirm `Win32_UI_Shell` is actively used by Task 1; keep it)

- [ ] **Step 1: Update `README.md:68-71`**

Replace the stale sentence claiming `cargo test --bin croft` and Windows CI are not wired up with an accurate statement that Windows CI runs `cargo fmt`, `cargo clippy`, `tests/platform`, `tests/cli`, and `cargo test --bin croft` on `x86_64-pc-windows-msvc` (with Unix-only daemon tests gated), keeping the link to `Windows_Port.md`.

- [ ] **Step 2: Verify `Cargo.toml` feature usage**

Confirm `ShellExecuteW` in `src/platform/os.rs` uses `Win32_UI_Shell` and `git diff README.md` is clean.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: update README Windows test and CI status"
```

---

### Task 15: Retarget self-update & prebuilt release URLs to `Zizeo/croft_windows` + Final Cross-Platform Verification (`#16`)

**Files:**
- Modify: `src/update_check.rs:28-60, 1234`
- Modify: `src/remote_prebuilt.rs:29, 64-66, 587`
- Modify: `Cargo.toml:7`
- Modify: `src/app/tests/updates.rs:509`

**Interfaces:**
- Produces: `pub const crate::update_check::GITHUB_REPO: &str = "Zizeo/croft_windows";`

- [ ] **Step 1: Update URL test assertions to fail first**

- `src/app/tests/updates.rs:509`: assert `"Open https://github.com/Zizeo/croft_windows/releases/tag/v9.9.9"`.
- `src/remote_prebuilt.rs:587`: assert `"https://github.com/Zizeo/croft_windows/.github/workflows/release.yml@refs/tags/v0.1.9"`.
- `src/update_check.rs:1234`: assert `"https://github.com/Zizeo/croft_windows/releases/download/v9/SHA256SUMS"`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --locked --bin croft remote_prebuilt::tests::the_release_identity && cargo test --locked --bin croft app::tests::updates::`
Expected: FAIL (`vitali87/croft` vs `Zizeo/croft_windows`).

- [ ] **Step 3: Centralize `GITHUB_REPO` and update `Cargo.toml`**

- In `src/update_check.rs`:
  ```rust
  pub const GITHUB_REPO: &str = "Zizeo/croft_windows";
  pub const RELEASES_LATEST_URL: &str =
      "https://api.github.com/repos/Zizeo/croft_windows/releases/latest";
  ```
  and use `format!("https://github.com/{GITHUB_REPO}/releases/tag/v{version}")` in `release_notes_url`.
- In `src/remote_prebuilt.rs`:
  - Derive `RELEASES` / `release_urls` / `bundle_url` / `release_identity` from `crate::update_check::GITHUB_REPO` (e.g., `const RELEASES: &str = "https://github.com/Zizeo/croft_windows/releases/download";` or `format!("https://github.com/{}/releases/download/...", crate::update_check::GITHUB_REPO)` and `format!("https://github.com/{}/.github/workflows/release.yml@refs/tags/v{version}", crate::update_check::GITHUB_REPO)`).
- In `Cargo.toml:7`: update `repository = "https://github.com/Zizeo/croft_windows"`.
- Leave historical issue comments (`vitali87/croft#...`) and unrelated mock fixtures (`review_threads.rs`, `git/tests.rs`) untouched.

- [ ] **Step 4: Run targeted tests and commit**

Run: `cargo test --locked --bin croft remote_prebuilt::tests && cargo test --locked --bin croft update_check::tests && cargo test --locked --bin croft app::tests::updates`
Expected: PASS.

```bash
git add src/update_check.rs src/remote_prebuilt.rs Cargo.toml src/app/tests/updates.rs
git commit -m "chore: point self-update and release URLs to Zizeo/croft_windows"
```

- [ ] **Step 5: Run full Windows + WSL Linux + Python verification gates**

1. Windows native:
   ```powershell
   cargo fmt --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --locked --test platform
   cargo test --locked --test cli
   cargo test --locked --bin croft
   ```
2. WSL Linux + repo contract checks (note literal `/home/elio/.cache/croft-linux-target`!):
   ```powershell
   wsl bash -lc "export PATH=/home/elio/.cargo/bin:\$PATH CARGO_TARGET_DIR=/home/elio/.cache/croft-linux-target && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked --test platform && cargo test --locked --test cli && python3 scripts/check_doc_ownership.py origin/main HEAD && python3 scripts/check_binstall_contract.py"
   ```
3. Verify `git status --short` shows only `ci-failure/` untracked.
