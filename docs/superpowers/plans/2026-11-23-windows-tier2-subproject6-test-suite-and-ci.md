# Windows Port Tier 2 — Sub-project 6: Full Unit Test Suite & Windows CI

> **For agentic workers:** REQUIRED: Use superpowers:executing-plans or superpowers:subagent-driven-development to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cargo check --bin croft --tests` and `cargo test --bin croft` compile and pass on Windows (`x86_64-pc-windows-msvc`), and add a Windows CI job to `.github/workflows/ci.yml`.

**Architecture:**
1. Gate genuinely Unix-only tests, fixtures (`#!/bin/sh`, `libc::getsid`, `libc::kill`, `libc::mkfifo`, `std::os::unix::fs::symlink`, Unix `0o600`/`0o755` permission-bit assertions), and Tier-3 module test suites (`gui_path`, `pair`) with `#[cfg(unix)]` / `#[cfg(all(test, unix))]` while keeping cross-platform round-trip tests active on Windows.
2. Add `LocalStream::pair()` to `src/platform/ipc.rs`, un-gate `crate::web::tls::{terminate, flush}` over `LocalStream`, and migrate in-process IPC tests (`src/app/tests/{mod.rs,agents.rs,files.rs,misc.rs}`) from `UnixListener`/`UnixStream` to `crate::platform::ipc::{LocalListener, LocalStream}`.
3. Update `src/clipboard.rs` `other_os_backend_tests` to exclude `target_os = "windows"` (since Windows has a native Win32 clipboard backend).
4. Resolve any Windows runtime test failures discovered when running `cargo test --bin croft`, then wire the `windows` job into `.github/workflows/ci.yml`.

---

## Task 1: Cross-Platform Test Primitives & Clipboard Gate
- [ ] Update `src/clipboard.rs` `other_os_backend_tests` cfg guard to exclude `target_os = "windows"`.
- [ ] Add `pub fn pair() -> io::Result<(Self, Self)>` to `win::LocalStream` in `src/platform/ipc.rs`.
- [ ] Replace `UnixStream` with `crate::platform::ipc::LocalStream` in `src/web/tls.rs` and remove `#[cfg(unix)]` from `terminate` / `flush` / `a_large_write_passes_through_whole`.

## Task 2: Group A — Infrastructure & Tier-3 Module Tests
- [ ] `src/gui_path.rs`: Gate `is_stripped`, `merge`, and `mod tests` to `unix`.
- [ ] `src/pair.rs`: Gate `mod tests` to `#[cfg(all(test, unix))]`.
- [ ] `src/prefs.rs`: Support `ERROR_DISK_FULL` (`112`) on `#[cfg(windows)]` in `write_aside` test hook.
- [ ] `src/hot_exit.rs` & `src/session_state.rs`: Guard `PermissionsExt` `0o600` assertions with `#[cfg(unix)]`.
- [ ] `src/review_ops.rs`: Gate `fake_gh` and `#!/bin/sh`-based tests with `#[cfg(unix)]`.
- [ ] `src/mcp/transport.rs`, `src/dap/transport.rs`, `src/dap/session/tests.rs`: Gate Unix `getsid`/`kill`/`symlink` tests with `#[cfg(unix)]`.
- [ ] `src/lsp/install.rs` & `src/lsp/manager/tests.rs`: Use `std::os::windows::process::ExitStatusExt` in `fake_output`, `executable_in_dir` in `python_for_stub_server`, and `#[cfg(unix)]` on Unix permission/PID tests.

## Task 3: Group B — Individual Module Tests
- [ ] Gate Unix symlink / mode-bit / shell-script tests in `src/config_layers.rs`, `src/cpp_build.rs`, `src/notebook_kernel.rs`, `src/tasks.rs`, `src/update_check.rs`, `src/workspace.rs`, `src/remote/tests.rs`, `src/testing/worker/tests.rs`, `src/widgets/editor/tests.rs`, `src/widgets/search/tests.rs`, and `src/widgets/terminal/tests.rs`.
- [ ] Update `src/notebook_kernel.rs` `candidates_prefer_the_workspace_venv` expectation for Windows (`python`).

## Task 4: Group C — `src/app/tests/*` and `tests/cli.rs`
- [ ] Migrate `UnixListener`/`UnixStream` IPC tests in `src/app/tests/{mod.rs,agents.rs,files.rs,misc.rs}` to `crate::platform::ipc::{LocalListener, LocalStream}`, guarding only `fcntl`/`mkfifo`/`symlink` checks with `#[cfg(unix)]`.
- [ ] Guard `PermissionsExt`, `#!/bin/sh` stub scripts, symlinks, and Tier-3 `collab` tests in `src/app/tests/{mod.rs,codeql.rs,files.rs,hot_exit.rs,misc.rs,notebook.rs,palette.rs,remote.rs,review.rs,sarif.rs,scm.rs,tasks.rs,testing.rs}` and `tests/cli.rs` with `#[cfg(unix)]`.

## Task 5: Full Compilation, Runtime Fixes, & Windows CI
- [ ] Verify `cargo check --bin croft --tests` and `cargo check --tests` pass with zero errors/warnings.
- [ ] Run `cargo test --test platform` and `cargo test --bin croft`, fixing any remaining Windows runtime issues in unit tests.
- [ ] Add the `windows` (`windows-latest`) job to `.github/workflows/ci.yml`.
- [ ] Format modified `.rs` files with `rustfmt --edition 2024`, re-verify checks and tests, and commit on `windows-port`.
