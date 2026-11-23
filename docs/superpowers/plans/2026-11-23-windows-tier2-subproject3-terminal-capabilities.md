# Sub-project 3 — Host Terminal Capabilities (Windows Terminal & WezTerm) Implementation Plan

> **For agentic workers:** Implement each step carefully, keep all `unsafe` Win32 console FFI confined to `src/platform/terminal.rs`, preserve `#[cfg(unix)]` behavior, and verify via `rustfmt`, `cargo check --bin croft`, and `cargo test --test platform`.

**Goal:** Enable host terminal capability probing (DA1 Sixel detection, Kitty keyboard protocol query, and XTWINOPS `CSI 16 t` / `CSI 14 t` cell pixel sizing) plus Windows Console font pixel fallback (`GetCurrentConsoleFontEx`) on Windows so Windows Terminal and WezTerm render inline graphics and handle keyboard disambiguation without Unix-only `#[cfg(unix)]` gates.

**Architecture:**
- Add `"Win32_System_Console"` to `windows-sys` in `Cargo.toml`.
- Introduce `src/platform/terminal.rs` exposing cross-platform response parsers (`HostProbeResult`, `has_complete_da1`, `parse_host_probe_bytes`, `parse_host_probe_bytes_with_grid`) and Windows-only console probing (`console_grid_size`, `console_font_cell_pixel`, `PendingWindowsProbe`, `ask_host_terminal`) that saves/restores console modes, writes VT probe sequences (`\x1b[?u\x1b[16t\x1b[14t\x1b[c`), reads VT reply sequences with `WaitForSingleObject` + `ReadConsoleInputW` within a 150ms deadline while preserving non-ESC user key events, and falls back gracefully on piped/non-console handles.
- Expose `ask_host()` / `PendingHostProbe` (with `answer()` and `answer_full()`) on both `#[cfg(unix)]` and `#[cfg(windows)]` in `src/iterm2_inline.rs`.
- Ungate `SIXEL_HOST` in `src/widgets/terminal.rs` and `StartupProbe` in `src/app/mod.rs`, applying probed `cell_pixel_size` to `app.cell_pixel`.
- Update `App::init_graphics` in `src/app/inline_images.rs` to fall back cleanly from `crossterm::terminal::window_size()` to `crossterm::terminal::size()`, `self.cell_pixel`, `console_font_cell_pixel()`, and `FALLBACK_CELL_PIXEL`.
- Add parser and non-console safety tests in `tests/platform.rs`.

---

## Step 1: Dependencies & Platform Module Export

- [x] Save plan document.
- [x] Add `"Win32_System_Console"` to `windows-sys` features in `Cargo.toml`.
- [x] Export `pub mod terminal;` in `src/platform/mod.rs`.

## Step 2: `src/platform/terminal.rs`

- [x] Implement `HostProbeResult`, `has_complete_da1`, `parse_host_probe_bytes`, `parse_host_probe_bytes_with_grid`.
- [x] Implement `console_font_cell_pixel` (`#[cfg(windows)]` via `GetCurrentConsoleFontEx`, returning `None` on non-Windows or non-console handles), `console_grid_size`, `PendingWindowsProbe` (with `Drop` safety restoring saved console modes), and `ask_host_terminal`.
- [x] Ensure non-console handles (`GetConsoleMode` failure) return immediately without blocking.

## Step 3: Wire Into `iterm2_inline.rs`, `terminal.rs`, `app/mod.rs`, `app/inline_images.rs`

- [x] Extend `src/iterm2_inline.rs` with `PendingHostProbe::answer_full` (`#[cfg(unix)]`) and `#[cfg(windows)]` `ask_host()` / `PendingHostProbe`.
- [x] Remove `#[cfg(unix)]` from `pub use images::SIXEL_HOST;` in `src/widgets/terminal.rs`.
- [x] Remove `#[cfg(unix)]` from `StartupProbe` and its startup wiring in `src/app/mod.rs`, and seed `app.cell_pixel` from `host.cell_pixel_size`.
- [x] Update `App::init_graphics` in `src/app/inline_images.rs` with grid + `cell_pixel` / `console_font_cell_pixel()` / `FALLBACK_CELL_PIXEL` fallback.

## Step 4: Tests in `tests/platform.rs`

- [x] Add `terminal_probe_parser_decodes_kitty_xtwinops_16t_14t_and_da1`.
- [x] Add `terminal_probe_safe_on_non_console_handles`.

## Step 5: Format, Verify, Commit

- [x] Run `rustfmt --edition 2024` on touched `.rs` files.
- [x] Run `cargo check --bin croft`.
- [x] Run `cargo test --test platform`.
- [ ] Commit changes on `windows-port` (excluding unrelated pre-existing `src/platform/fs.rs` diff).
