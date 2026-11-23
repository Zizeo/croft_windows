# Sub-project 5 — Local Windows IPC (`croft view`, `croft edit`, `croft hook`) & `croft://` Links Implementation Plan

> **For agentic workers:** Implement each step carefully, verify via `rustfmt`, `cargo check --bin croft`, and `cargo test --test platform`, and keep unrelated diffs untouched.

**Goal:** Enable `croft view`, `croft edit [--wait]`, `croft hook`, `croft decide`, and `croft://` links on Windows using capability-authenticated loopback TCP in `src/platform/ipc.rs` and `HKCU\Software\Classes\croft` registration in `src/cli/links.rs`.

---

## Steps

### Step 1: Capability-Authenticated Loopback TCP (`src/platform/ipc.rs`)
- [x] Implement `LocalListener` (`bind`, `set_nonblocking`, `local_addr`, `accept`, `Drop`) and `LocalStream` (`connect`, `set_nonblocking`, `set_read_timeout`, `set_write_timeout`, `try_clone`, `shutdown`, `Read`/`Write` for owned & shared refs) on `#[cfg(windows)]`.

### Step 2: Cross-Platform `view_ipc` & Host Listener (`src/view_ipc.rs`, `src/app/host_integration.rs`)
- [x] Enable `send`, `run`, `edit`, `stage_stdin`, `stage_at`, `read_line_by_deadline`, `read_request`, `write_reply`, and cross-platform `Created`/`take_back` in `src/view_ipc.rs`.
- [x] Enable `prepare_view_listener`, `bind_view_socket`, `drain_view_requests`, `answer_view_client`, `apply_view_request`, `probe_view_path`, `sweep_dead_view_sockets`, and `sweep_staged_stdin` in `src/app/host_integration.rs`.
- [x] Update non-Unix `PaneClient` placeholder in `src/pane_host.rs` to `PaneClient(std::convert::Infallible)`.

### Step 3: Agent Edit Approval Hooks (`src/agent_approval.rs`, `src/agent_hook.rs`)
- [x] Enable `bind`, `Pending::answer`, `accept_into`, `DecideReply`, `decide_by_token`, and `send_decision` in `src/agent_approval.rs`.
- [x] Enable `connect_croft` and `exchange` in `src/agent_hook.rs`.

### Step 4: Windows `croft://` Links (`src/deep_link.rs`, `src/cli/links.rs`)
- [x] Add `windows_registry_command` in `src/deep_link.rs`.
- [x] Implement cross-platform `open_link` (with `platform::env::hostname` + synchronous `Command::status` on non-Unix) and `#[cfg(windows)] install_link_handler` (`reg.exe add` under `HKCU\Software\Classes\croft`) in `src/cli/links.rs`.

### Step 5: Tests, Verification, Commit
- [x] Add `ipc_local_listener_nonblocking_roundtrip_auth_and_cleanup` in `tests/platform.rs`.
- [x] Run `rustfmt --edition 2024`, `cargo check --bin croft`, `cargo test --test platform`.
- [x] Commit Sub-project 5 changes on branch `windows-port`.
