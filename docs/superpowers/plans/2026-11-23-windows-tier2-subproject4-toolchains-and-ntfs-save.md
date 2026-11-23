# Sub-project 4 — Windows Toolchains (DAP, Venvs, Task Binaries, & NTFS Save Hardening) Implementation Plan

> **For agentic workers:** Carry out each step carefully, verify via `rustfmt`, `cargo check --bin croft`, and `cargo test --test platform`, and keep unrelated diffs untouched.

**Goal:** Make DAP tool resolution (`debug-venv\Scripts\python.exe`, `uv`, `node`, `dlv.exe`, `DLV_INSTALL`, `lldb-dap.exe`), workspace Python `.venv` discovery, `node_modules/.bin/tsc.cmd` lookup, and NTFS save/rename sharing-violation retries work natively on Windows.

---

## Steps

### Step 1: Platform Env Helpers (`src/platform/env.rs`)
- [x] Add `venv_python_path`, `venv_python_in`, `find_dlv_in`, `lldb_dap_rank`, `find_lldb_dap_on_path`.

### Step 2: `uv` Visibility & `src/dap/install.rs`
- [x] Make `uv_program` and `ensure_uv` `pub(crate)` in `src/lsp/install.rs`.
- [x] Update `debug_venv_python`, `ensure_debug_venv`, `which`, `find_dlv`, `DLV_INSTALL` (`DLV_INSTALL_WINDOWS` / `DLV_INSTALL_UNIX`), and unit test in `src/dap/install.rs`.

### Step 3: Workspace `.venv`, `lldb-dap`, and `tsc.cmd` Resolution
- [x] Update `src/app/debugger/adapters.rs` (`lldb_dap_rank`, `lldb_dap_path` with Program Files LLVM fallback, `lldb_dap_missing_message`).
- [x] Update `src/app/debugger/python.rs` (`project_python_on`, `find_python_venv`).
- [x] Update `src/notebook_kernel.rs` (`python_candidates`).
- [x] Update `src/testing/worker/pytest.rs` (`pytest_launch_on`).
- [x] Update `src/app/bottom_panel.rs` (`project_check_command` local `tsc.cmd` resolution).

### Step 4: NTFS Save Contention Retry (`src/platform/fs.rs`, `src/prefs.rs`, `src/widgets/editor/viewers.rs`)
- [x] Add `NTFS_RETRY_BACKOFF_MS`, `is_transient_ntfs_contention`, `retry_transient_io`, `rename_retrying`, `write_retrying` in `src/platform/fs.rs`.
- [x] Wire `rename_retrying`, `write_retrying`, and `retry_transient_io` into `src/prefs.rs` and `src/widgets/editor/viewers.rs` (plus `drop(file)` before metadata update in `write_aside`).

### Step 5: Tests, Verification, Commit
- [x] Add toolchain and NTFS retry tests in `tests/platform.rs`.
- [x] Run `rustfmt --edition 2024`, `cargo check --bin croft`, `cargo test --test platform`.
- [x] Commit Sub-project 4 changes on branch `windows-port`.
