# Windows Tier 2 Port — Sub-project 1 (Compilable Core & Platform Layer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `croft` compile cleanly on `x86_64-pc-windows-msvc` and run natively on Windows with a dedicated `src/platform/` OS layer, Win32 clipboard, Windows config/cache/state paths, clean Tier-3 Unix-only module gating, and a minimal working ConPTY PowerShell terminal pane.

**Architecture:** Introduce a focused `src/platform/` abstraction (`env`, `fs`, `process`, `time`, `os`, `clipboard`, `ipc`) with `#[cfg(unix)]` (preserving existing Unix behavior verbatim) and `#[cfg(windows)]` (`windows-sys` + `dunce` + stdlib `File::lock`) backends, tested independently via `tests/platform.rs` (`cargo test --test platform`) so TDD cycles take ~2 seconds without compiling legacy Unix-only `#[cfg(test)]` modules. Gate Tier-3 Unix-socket subsystems at their module boundaries so `App` stays clean, and add a channel-driven blocking ConPTY reader in `src/widgets/terminal/backend.rs` so `App::new()` launches with a working basic PowerShell pane.

**Tech Stack:** Rust 1.97.1 (Edition 2024), `windows-sys` 0.59, `dunce` 1, `portable-pty` 0.9, `ratatui` 0.30, `crossterm` 0.29, `alacritty_terminal` 0.26.

## Global Constraints

- **Toolchain:** Rust `1.97.1`, Edition `2024` (`rust-toolchain.toml`, `Cargo.toml`).
- **Targets:** Must compile and run on `x86_64-pc-windows-msvc` while preserving full `#[cfg(unix)]` compatibility (Linux and macOS code paths must remain intact).
- **Shells & Terminals:** Must support PowerShell 7+ (`pwsh.exe`), Windows PowerShell 5.1 (`powershell.exe`), and `cmd.exe` fallback, hosted in Windows Terminal, WezTerm, or any 3rd-party terminal emulator (with text-glyph fallback on legacy `conhost.exe`).
- **Dependencies:** Move `libc = "0.2"` and `async-lsp`'s `"stdio"` feature to `[target.'cfg(unix)'.dependencies]`; add only `windows-sys = "0.59"` (minimal feature set) and `dunce = "1"` under `[target.'cfg(windows)'.dependencies]`; use Rust stdlib `File::lock` / `File::try_lock` instead of an external locking crate.
- **Safety boundary:** All `unsafe` Win32 FFI must live exclusively inside `src/platform/` and `src/clipboard.rs` — zero direct `windows_sys` calls in `src/app/` or `src/widgets/`.
- **CLI parity:** Unsupported Tier-3 subcommands remain registered in `clap` (`src/cli/commands.rs`) and return a clear, actionable runtime error on Windows rather than being removed from `--help`.

---

### Task 1: Build System & Dependencies (`Cargo.toml`, `build.rs`)

**Files:**
- Modify: `Cargo.toml:36-42`, `Cargo.toml:129`, `Cargo.toml:280-287`
- Modify: `build.rs:119-129`

**Interfaces:**
- Consumes: none
- Produces: Windows-compatible `Cargo.toml` (`windows-sys`, `dunce`, Unix-gated `libc` and `async-lsp/stdio`), pure-Rust UTC ISO-8601 `CROFT_BUILD_TIME` in `build.rs`.

- [ ] **Step 1: Update `Cargo.toml` target-specific dependencies**

In `Cargo.toml`:
1. Remove `libc = "0.2"` from `[dependencies]` (around line 40).
2. Change line 129 from:
```toml
async-lsp = { version = "0.2", default-features = false, features = ["omni-trait", "stdio", "tracing", "tokio"] }
```
to:
```toml
async-lsp = { version = "0.2", default-features = false, features = ["omni-trait", "tracing", "tokio"] }
```
3. Under the `[target.'cfg(not(target_os = "android"))'.dependencies]` section (after line 286), add:
```toml
[target.'cfg(unix)'.dependencies]
libc = "0.2"
async-lsp = { version = "0.2", default-features = false, features = ["stdio"] }

[target.'cfg(windows)'.dependencies]
dunce = "1"
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_System_DataExchange",
    "Win32_System_JobObjects",
    "Win32_System_Memory",
    "Win32_System_SystemInformation",
    "Win32_System_Threading",
    "Win32_System_Time",
    "Win32_UI_Shell",
] }
```

- [ ] **Step 2: Replace Unix `date` subprocess in `build.rs` with pure-Rust UTC formatting**

Replace `emit_build_time()` in `build.rs:119-129` with:

```rust
fn emit_build_time() {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| format_utc_iso8601(d.as_secs()))
        .unwrap_or_else(|| String::from("unknown"));
    println!("cargo:rustc-env=CROFT_BUILD_TIME={time}");
}

/// Convert Unix epoch seconds to `YYYY-MM-DDTHH:MM:SSZ` without spawning `date`.
/// Uses Howard Hinnant's civil-from-days algorithm.
fn format_utc_iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}
```

- [ ] **Step 3: Verify dependency resolution**

Run: `cargo metadata --format-version 1 | Out-Null; if ($LASTEXITCODE -eq 0) { "metadata OK" }`
Expected: `metadata OK` and `Cargo.lock` updated with `dunce` and `windows-sys`.

- [ ] **Step 4: Commit**

```powershell
git add Cargo.toml Cargo.lock build.rs
git commit -m "build: gate unix deps and replace build.rs date subprocess with pure Rust UTC"
```

---

### Task 2: `src/platform/` Core (`env`, `time`, `os`) & Standalone Test Harness (`tests/platform.rs`)

**Files:**
- Create: `src/platform/mod.rs`
- Create: `src/platform/env.rs`
- Create: `src/platform/time.rs`
- Create: `src/platform/os.rs`
- Create: `tests/platform.rs`

**Interfaces:**
- Consumes: `windows-sys` (`Win32_Foundation`, `Win32_System_SystemInformation`, `Win32_System_Threading`, `Win32_System_Time`)
- Produces:
  - `platform::env::{home_dir, config_dir, state_dir, cache_dir, hostname, default_shell}`
  - `platform::time::{LocalDateTime, local_datetime, thread_cpu_time}`
  - `platform::os::{reveal_in_file_manager}`

- [ ] **Step 1: Create `tests/platform.rs` with failing tests for `env`, `time`, and `os`**

Create `tests/platform.rs`:
```rust
#![allow(dead_code)]

#[path = "../src/platform/mod.rs"]
mod platform;

#[test]
fn env_home_and_dirs_resolve_non_empty() {
    let home = platform::env::home_dir();
    assert!(home.is_some(), "home_dir() must resolve on this OS");
    assert!(platform::env::config_dir().ends_with("croft"));
    assert!(platform::env::state_dir().is_absolute() || platform::env::state_dir().ends_with("croft"));
    assert!(platform::env::cache_dir().is_absolute() || platform::env::cache_dir().ends_with("croft"));
    assert!(!platform::env::hostname().is_empty());
    assert!(!platform::env::default_shell().is_empty());
}

#[test]
fn env_windows_config_honors_xdg_then_appdata() {
    use std::ffi::OsString;
    use std::path::PathBuf;
    let xdg = platform::env::config_dir_windows(
        Some(OsString::from(r"C:\custom\xdg")),
        Some(OsString::from(r"C:\Users\u\AppData\Roaming")),
        Some(PathBuf::from(r"C:\Users\u")),
    );
    assert_eq!(xdg, PathBuf::from(r"C:\custom\xdg").join("croft"));

    let appdata = platform::env::config_dir_windows(
        None,
        Some(OsString::from(r"C:\Users\u\AppData\Roaming")),
        Some(PathBuf::from(r"C:\Users\u")),
    );
    assert_eq!(
        appdata,
        PathBuf::from(r"C:\Users\u\AppData\Roaming").join("croft")
    );

    let state = platform::env::state_dir_windows(
        None,
        Some(OsString::from(r"C:\Users\u\AppData\Local")),
        Some(PathBuf::from(r"C:\Users\u")),
    );
    assert_eq!(
        state,
        PathBuf::from(r"C:\Users\u\AppData\Local").join("croft").join("state")
    );

    let cache = platform::env::cache_dir_windows(
        Some(OsString::from(r"C:\Users\u\AppData\Local")),
        Some(PathBuf::from(r"C:\Users\u")),
    );
    assert_eq!(
        cache,
        PathBuf::from(r"C:\Users\u\AppData\Local").join("croft").join("cache")
    );
}

#[test]
fn time_local_datetime_and_thread_cpu() {
    let dt = platform::time::local_datetime(1_700_000_000).expect("valid epoch timestamp");
    assert!((2023..=2024).contains(&dt.year));
    assert!((1..=12).contains(&dt.month));
    assert!((1..=31).contains(&dt.day));
    assert!(dt.weekday <= 6);
    assert!(dt.hour <= 23);
    assert!(dt.minute <= 59);
    assert!(dt.second <= 59);
    assert!((-720..=840).contains(&dt.utc_offset_mins));

    let cpu = platform::time::thread_cpu_time();
    assert!(cpu.is_some(), "thread_cpu_time should return Some");
}
```

- [ ] **Step 2: Run test to verify it fails (files don't exist yet)**

Run: `cargo test --test platform`
Expected: FAIL with missing `../src/platform/mod.rs`.

- [ ] **Step 3: Implement `src/platform/mod.rs`, `env.rs`, `time.rs`, and `os.rs`**

Create `src/platform/mod.rs`:
```rust
//! Cross-platform OS abstractions isolating Unix (`libc` / `std::os::unix`)
//! and Windows (`windows-sys` / `dunce`) primitives behind one API.

pub mod env;
pub mod os;
pub mod time;
```

Create `src/platform/env.rs`:
```rust
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// User home directory: `HOME` on Unix; `HOME` -> `USERPROFILE` ->
/// `HOMEDRIVE` + `HOMEPATH` on Windows.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(h));
    }
    #[cfg(windows)]
    {
        if let Some(up) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(up));
        }
        if let (Some(drive), Some(path)) = (
            std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty()),
            std::env::var_os("HOMEPATH").filter(|v| !v.is_empty()),
        ) {
            let mut full = drive;
            full.push(path);
            return Some(PathBuf::from(full));
        }
    }
    None
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        config_dir_windows(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("APPDATA"),
            home_dir(),
        )
    }
    #[cfg(not(windows))]
    {
        config_dir_unix(std::env::var_os("XDG_CONFIG_HOME"), home_dir())
    }
}

pub fn state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        state_dir_windows(
            std::env::var_os("XDG_STATE_HOME"),
            std::env::var_os("LOCALAPPDATA"),
            home_dir(),
        )
    }
    #[cfg(not(windows))]
    {
        state_dir_unix(std::env::var_os("XDG_STATE_HOME"), home_dir())
    }
}

pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        cache_dir_windows(std::env::var_os("LOCALAPPDATA"), home_dir())
    }
    #[cfg(not(windows))]
    {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
            .join("croft")
    }
}

pub fn config_dir_unix(xdg_config_home: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("croft")
}

pub fn state_dir_unix(xdg_state_home: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(xdg) = xdg_state_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return xdg.join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("croft")
}

pub fn config_dir_windows(
    xdg_config_home: Option<OsString>,
    appdata: Option<OsString>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("croft");
    }
    if let Some(roaming) = appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(roaming).join("croft");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("croft")
}

pub fn state_dir_windows(
    xdg_state_home: Option<OsString>,
    local_appdata: Option<OsString>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(xdg) = xdg_state_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return xdg.join("croft");
    }
    if let Some(local) = local_appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(local).join("croft").join("state");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("croft")
}

pub fn cache_dir_windows(local_appdata: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(local) = local_appdata.filter(|v| !v.is_empty()) {
        return PathBuf::from(local).join("croft").join("cache");
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("croft")
}

pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        if ok {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = String::from_utf8_lossy(&buf[..end]).into_owned();
            if !s.is_empty() {
                return s;
            }
        }
        String::from("local")
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{
            ComputerNameDnsHostname, GetComputerNameExW,
        };
        let mut len: u32 = 0;
        unsafe {
            GetComputerNameExW(ComputerNameDnsHostname, std::ptr::null_mut(), &mut len);
        }
        if len > 0 {
            let mut buf = vec![0u16; len as usize];
            let ok = unsafe {
                GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len)
            };
            if ok != 0 {
                let s = String::from_utf16_lossy(&buf[..len as usize]);
                if !s.is_empty() {
                    return s;
                }
            }
        }
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("local"))
    }
}

pub fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("/bin/bash"))
    }
    #[cfg(windows)]
    {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if let Some(found) = find_on_path(candidate) {
                return found.to_string_lossy().into_owned();
            }
        }
        std::env::var("COMSPEC")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("cmd.exe"))
    }
}

#[cfg(windows)]
fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let direct = Path::new(exe);
    direct.is_file().then(|| direct.to_path_buf())
}
```

Create `src/platform/time.rs`:
```rust
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 0 = Sunday ..= 6 = Saturday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Signed offset in minutes east of UTC (e.g. +120 for UTC+2).
    pub utc_offset_mins: i32,
}

#[cfg(unix)]
pub fn local_datetime(unix_secs: i64) -> Option<LocalDateTime> {
    #[allow(deprecated)]
    let secs = unix_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some(LocalDateTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        weekday: tm.tm_wday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
        utc_offset_mins: (tm.tm_gmtoff / 60) as i32,
    })
}

#[cfg(windows)]
pub fn local_datetime(unix_secs: i64) -> Option<LocalDateTime> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        FileTimeToLocalFileTime, FileTimeToSystemTime,
    };
    const WINDOWS_EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    const TICKS_PER_SEC: i64 = 10_000_000;

    let ticks = unix_secs
        .checked_add(WINDOWS_EPOCH_DIFF_SECS)?
        .checked_mul(TICKS_PER_SEC)?;
    if ticks < 0 {
        return None;
    }
    let utc_u64 = ticks as u64;
    let utc_ft = FILETIME {
        dwLowDateTime: utc_u64 as u32,
        dwHighDateTime: (utc_u64 >> 32) as u32,
    };
    let mut local_ft: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToLocalFileTime(&utc_ft, &mut local_ft) } == 0 {
        return None;
    }
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&local_ft, &mut st) } == 0 {
        return None;
    }
    let local_u64 = (u64::from(local_ft.dwHighDateTime) << 32) | u64::from(local_ft.dwLowDateTime);
    let diff_ticks = local_u64 as i128 - utc_u64 as i128;
    let utc_offset_mins = (diff_ticks / (60 * TICKS_PER_SEC as i128)) as i32;

    Some(LocalDateTime {
        year: i32::from(st.wYear),
        month: u32::from(st.wMonth),
        day: u32::from(st.wDay),
        weekday: u32::from(st.wDayOfWeek),
        hour: u32::from(st.wHour),
        minute: u32::from(st.wMinute),
        second: u32::from(st.wSecond),
        utc_offset_mins,
    })
}

#[cfg(unix)]
pub fn thread_cpu_time() -> Option<Duration> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut ts) } == 0;
    ok.then(|| Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
}

#[cfg(windows)]
pub fn thread_cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    let to_u64 = |ft: FILETIME| (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    let total_100ns = to_u64(kernel).saturating_add(to_u64(user));
    Some(Duration::from_nanos(total_100ns.saturating_mul(100)))
}
```

Create `src/platform/os.rs`:
```rust
use std::path::Path;
use std::process::{Command, Stdio};

/// Reveal `path` in the OS file manager (Finder on macOS, Explorer on Windows).
/// Returns `Ok(label)` describing the manager, or `Err(label)` on failure.
pub fn reveal_in_file_manager(path: &Path) -> Result<&'static str, &'static str> {
    #[cfg(target_os = "macos")]
    {
        if Command::new("open")
            .arg("-R")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            Ok("Revealed in Finder")
        } else {
            Err("Could not reveal in Finder")
        }
    }
    #[cfg(windows)]
    {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        if Command::new("explorer.exe")
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            Ok("Revealed in File Explorer")
        } else {
            Err("Could not reveal in File Explorer")
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Err("Reveal in file manager is not supported on this OS")
    }
}
```

- [ ] **Step 4: Run `cargo test --test platform` to verify all tests pass**

Run: `cargo test --test platform`
Expected: PASS (3 passed, 0 failed).

- [ ] **Step 5: Commit**

```powershell
git add src/platform/mod.rs src/platform/env.rs src/platform/time.rs src/platform/os.rs tests/platform.rs
git commit -m "feat(platform): add env, time, and os abstractions with Windows backends"
```

---

### Task 3: `src/platform/fs.rs` and `src/platform/process.rs`

**Files:**
- Create: `src/platform/fs.rs`
- Create: `src/platform/process.rs`
- Modify: `src/platform/mod.rs`
- Modify: `tests/platform.rs`

**Interfaces:**
- Consumes: `dunce`, `windows-sys` (`Win32_Foundation`, `Win32_System_JobObjects`, `Win32_System_Threading`)
- Produces:
  - `platform::fs::{canonicalize, is_executable, set_private_file_mode, set_private_dir_mode, open_nofollow, try_lock_exclusive, lock_exclusive, create_file_link}`
  - `platform::process::{detach_from_tty, ManagedChild, is_pid_alive, terminate_pid, raise_fd_limit, reset_sigpipe_default}`

- [ ] **Step 1: Add failing tests for `platform::fs` and `platform::process` to `tests/platform.rs`**

Append to `tests/platform.rs`:
```rust
#[test]
fn fs_canonicalize_has_no_verbatim_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let canon = platform::fs::canonicalize(dir.path()).unwrap();
    let s = canon.to_string_lossy();
    assert!(!s.starts_with(r"\\?\"), "unexpected verbatim prefix: {s}");
}

#[test]
fn fs_executable_lock_and_link() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notes.txt");
    std::fs::write(&txt, b"hello").unwrap();
    assert!(!platform::fs::is_executable(&txt));

    #[cfg(windows)]
    {
        let cmd = dir.path().join("tool.cmd");
        std::fs::write(&cmd, b"@echo hi\r\n").unwrap();
        assert!(platform::fs::is_executable(&cmd));
    }

    let lock_path = dir.path().join("lock.file");
    let f1 = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    let f2 = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(platform::fs::try_lock_exclusive(&f1).unwrap());
    assert!(!platform::fs::try_lock_exclusive(&f2).unwrap());
    drop(f1);
    assert!(platform::fs::try_lock_exclusive(&f2).unwrap());

    let dst = dir.path().join("linked.txt");
    platform::fs::create_file_link(&txt, &dst).unwrap();
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello");
}

#[test]
fn process_liveness_and_managed_child_kill_tree() {
    assert!(platform::process::is_pid_alive(std::process::id()));

    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        c
    };
    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new("sleep");
        c.arg("30");
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut managed = platform::process::ManagedChild::spawn_in_group(&mut cmd).unwrap();
    let pid = managed.id();
    assert!(platform::process::is_pid_alive(pid));
    managed.kill_tree();
    assert!(!platform::process::is_pid_alive(pid));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test platform`
Expected: FAIL (`fs` and `process` modules not found).

- [ ] **Step 3: Implement `src/platform/fs.rs`, `src/platform/process.rs`, and export in `src/platform/mod.rs`**

Update `src/platform/mod.rs`:
```rust
pub mod env;
pub mod fs;
pub mod os;
pub mod process;
pub mod time;
```

Create `src/platform/fs.rs`:
```rust
use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::canonicalize(path)
    }
}

pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        let dot_ext = format!(".{}", ext.to_ascii_uppercase());
        let pathext = std::env::var("PATHEXT")
            .unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"));
        pathext
            .split(';')
            .any(|e| e.trim().eq_ignore_ascii_case(&dot_ext))
    }
}

pub fn set_private_file_mode(opts: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    {
        let _ = opts;
    }
}

pub fn set_private_dir_mode(builder: &mut DirBuilder) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    #[cfg(not(unix))]
    {
        let _ = builder;
    }
}

pub fn open_nofollow(opts: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let _ = opts;
    }
}

/// Attempt a non-blocking exclusive advisory lock on `file`.
/// Returns `Ok(true)` if acquired, `Ok(false)` if held by another opener.
pub fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Blocking exclusive lock on `file`.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    file.lock()
}

/// Link `link` to `target`: symlink on Unix; hard link with copy fallback on
/// Windows so unprivileged users never hit ERROR_PRIVILEGE_NOT_HELD.
pub fn create_file_link(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if std::fs::hard_link(target, link).is_ok() {
            return Ok(());
        }
        std::fs::copy(target, link).map(|_| ())
    }
}
```

Create `src/platform/process.rs`:
```rust
use std::io;
use std::process::{Child, Command};

/// Configure `cmd` so it detaches from croft's controlling terminal / console
/// window and cannot hijack raw-mode input or pop up a console window.
pub fn detach_from_tty(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
        };
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// A spawned child whose entire descendant process tree can be terminated
/// cleanly (Unix process group; Windows anonymous Job Object with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
pub struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    job: Option<WinHandle>,
}

#[cfg(windows)]
struct WinHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for WinHandle {}

#[cfg(windows)]
impl Drop for WinHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

impl ManagedChild {
    pub fn spawn_in_group(cmd: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
            let child = cmd.spawn()?;
            Ok(Self { child })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };
            use windows_sys::Win32::System::Threading::{
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
            };

            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            let child = cmd.spawn()?;
            let job = unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    None
                } else {
                    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                    let set_ok = SetInformationJobObject(
                        h,
                        JobObjectExtendedLimitInformation,
                        (&raw const info).cast(),
                        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    );
                    let assign_ok = if set_ok != 0 {
                        AssignProcessToJobObject(h, child.as_raw_handle())
                    } else {
                        0
                    };
                    if assign_ok != 0 {
                        Some(WinHandle(h))
                    } else {
                        windows_sys::Win32::Foundation::CloseHandle(h);
                        None
                    }
                }
            };
            Ok(Self { child, job })
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn into_child(self) -> Child {
        self.child
    }

    pub fn kill_tree(&mut self) {
        #[cfg(unix)]
        {
            if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job.as_ref() {
                unsafe {
                    windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0, 1);
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
}

pub fn terminate_pid(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        pid > 0 && unsafe { libc::kill(pid, libc::SIGTERM) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1);
            CloseHandle(h);
            ok != 0
        }
    }
}

pub fn raise_fd_limit() -> u64 {
    #[cfg(unix)]
    {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
            return 0;
        }
        let ceiling: libc::rlim_t = 1 << 20;
        let target = if lim.rlim_max == libc::RLIM_INFINITY {
            ceiling
        } else {
            lim.rlim_max.min(ceiling)
        };
        if lim.rlim_cur >= target {
            return lim.rlim_cur as u64;
        }
        let raised = libc::rlimit {
            rlim_cur: target,
            rlim_max: lim.rlim_max,
        };
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
            target as u64
        } else {
            lim.rlim_cur as u64
        }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

pub fn reset_sigpipe_default() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}
```

- [ ] **Step 4: Run `cargo test --test platform` to verify**

Run: `cargo test --test platform`
Expected: PASS (6 passed, 0 failed).

- [ ] **Step 5: Commit**

```powershell
git add src/platform/mod.rs src/platform/fs.rs src/platform/process.rs tests/platform.rs
git commit -m "feat(platform): add fs and process abstractions with Win32 Job Object backend"
```

---

### Task 4: `src/platform/clipboard.rs`, `src/platform/ipc.rs`, Wire `src/clipboard.rs` & `src/main.rs`

**Files:**
- Create: `src/platform/clipboard.rs`
- Create: `src/platform/ipc.rs`
- Modify: `src/platform/mod.rs`
- Modify: `src/clipboard.rs:1-90`
- Modify: `src/main.rs` (declare `mod platform;`)
- Modify: `tests/platform.rs`

**Interfaces:**
- Consumes: `windows-sys` (`Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_Foundation`)
- Produces:
  - `platform::clipboard::{encode_utf16_nul, decode_utf16_nul, read_string, write_string}`
  - `platform::ipc::{LocalListener, LocalStream}`

- [ ] **Step 1: Add failing UTF-16 clipboard codec test to `tests/platform.rs`**

Append to `tests/platform.rs`:
```rust
#[test]
fn clipboard_utf16_codec_roundtrip_and_nul_rejection() {
    for sample in ["", "hello world", "café naïve", "🚀🦀 Windows\r\nline2"] {
        let encoded = platform::clipboard::encode_utf16_nul(sample).unwrap();
        assert_eq!(encoded.last(), Some(&0));
        let decoded = platform::clipboard::decode_utf16_nul(&encoded);
        assert_eq!(decoded, sample);
    }
    assert!(platform::clipboard::encode_utf16_nul("a\0b").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test platform`
Expected: FAIL (`clipboard` module not found).

- [ ] **Step 3: Implement `src/platform/clipboard.rs`, `src/platform/ipc.rs`, and wire `src/clipboard.rs` & `src/main.rs`**

Update `src/platform/mod.rs`:
```rust
pub mod clipboard;
pub mod env;
pub mod fs;
pub mod ipc;
pub mod os;
pub mod process;
pub mod time;
```

Create `src/platform/clipboard.rs`:
```rust
/// Encode UTF-8 text into NUL-terminated UTF-16LE code units for Win32
/// `CF_UNICODETEXT`. Returns `None` if `text` contains an embedded NUL.
pub fn encode_utf16_nul(text: &str) -> Option<Vec<u16>> {
    if text.as_bytes().contains(&0) {
        return None;
    }
    let mut out: Vec<u16> = text.encode_utf16().collect();
    out.push(0);
    Some(out)
}

/// Decode a NUL-terminated UTF-16 slice back into UTF-8.
pub fn decode_utf16_nul(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(windows)]
struct OpenClipboardGuard;

#[cfg(windows)]
impl OpenClipboardGuard {
    fn open() -> Option<Self> {
        use windows_sys::Win32::System::DataExchange::OpenClipboard;
        for attempt in 0..5 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Self);
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        None
    }
}

#[cfg(windows)]
impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
    }
}

#[cfg(windows)]
pub fn read_string() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;

    let _guard = OpenClipboardGuard::open()?;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return None;
    }
    let byte_len = unsafe { GlobalSize(handle) };
    if byte_len < 2 {
        return Some(String::new());
    }
    let ptr = unsafe { GlobalLock(handle) } as *const u16;
    if ptr.is_null() {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, byte_len / 2) };
    let text = decode_utf16_nul(slice);
    unsafe {
        GlobalUnlock(handle);
    }
    Some(text)
}

#[cfg(windows)]
pub fn write_string(text: &str) -> bool {
    use windows_sys::Win32::System::DataExchange::{EmptyClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock,
    };
    const CF_UNICODETEXT: u32 = 13;

    let Some(units) = encode_utf16_nul(text) else {
        return false;
    };
    let Some(_guard) = OpenClipboardGuard::open() else {
        return false;
    };
    if unsafe { EmptyClipboard() } == 0 {
        return false;
    }
    let bytes = units.len() * std::mem::size_of::<u16>();
    let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
    if hmem.is_null() {
        return false;
    }
    let ptr = unsafe { GlobalLock(hmem) } as *mut u16;
    if ptr.is_null() {
        unsafe {
            GlobalFree(hmem);
        }
        return false;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
        GlobalUnlock(hmem);
        if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
            GlobalFree(hmem);
            return false;
        }
    }
    true
}
```

Create `src/platform/ipc.rs`:
```rust
//! Cross-platform local IPC stream/listener types.
//! On Unix these alias `std::os::unix::net`. On Windows (Sub-project 1), they
//! are inert placeholders so `App` compiles cleanly; Sub-project 5 replaces
//! them with Named Pipes / Windows `AF_UNIX`.

#[cfg(unix)]
pub type LocalListener = std::os::unix::net::UnixListener;
#[cfg(unix)]
pub type LocalStream = std::os::unix::net::UnixStream;

#[cfg(windows)]
#[derive(Debug)]
pub struct LocalListener;

#[cfg(windows)]
#[derive(Debug)]
pub struct LocalStream;
```

Update `src/clipboard.rs`:
- In `read_string()` (lines 22–30), add:
```rust
    #[cfg(target_os = "windows")]
    {
        crate::platform::clipboard::read_string()
    }
```
and change the fallback cfg from `#[cfg(not(any(target_os = "macos", target_os = "linux")))]` to `#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]`.
- In `write_string()` (lines 78–86), add:
```rust
    #[cfg(target_os = "windows")]
    {
        crate::platform::clipboard::write_string(text)
    }
```
and update the fallback cfg to exclude `"windows"`.

In `src/main.rs`, add `mod platform;` alphabetically (right after `mod permalink;`).

- [ ] **Step 4: Run `cargo test --test platform`**

Run: `cargo test --test platform`
Expected: PASS (7 passed, 0 failed).

- [ ] **Step 5: Commit**

```powershell
git add src/platform/mod.rs src/platform/clipboard.rs src/platform/ipc.rs src/clipboard.rs src/main.rs tests/platform.rs
git commit -m "feat(platform): add Win32 clipboard and IPC boundary types"
```

---

### Task 5: Migrate Paths, `HOME`, `canonicalize`, `localtime_r`, `hostname`, `thread_cpu`, & File Permissions/Locks

**Files:**
- Modify: `src/paths.rs` (delegate to `crate::platform::env`)
- Modify: `src/snippets.rs:285-315`, `src/widgets/codeql.rs:430-445`, `src/widgets/terminal/clock.rs:155-171`, `src/widgets/file_tree/android_trash.rs:100-130` (use `platform::time::local_datetime`, gate `EXDEV` under `#[cfg(unix)]`)
- Modify: `src/command_history.rs:40-55`, `src/session_host/identity.rs:48-64` (use `platform::env::hostname()`)
- Modify: `src/startup_trace.rs:184-194` (use `platform::time::thread_cpu_time()`)
- Modify: `src/app/debugger/adapters.rs:75-85`, `src/app/code_scanning.rs:1475-1485`, `src/app/codeql/contextual.rs:124-135`, `src/app/codeql/run.rs:118-130`, `src/app/runners.rs:2605-2630`, `src/app/save_restore.rs:2740-2760`, `src/app/terminal_panes.rs:1975-2040`, `src/codeql_cli.rs:165-195`, `src/config_layers.rs:770-785`, `src/history.rs:255-340`, `src/hot_exit.rs:165-180`, `src/live_run.rs:370-385`, `src/notebook.rs:155-170`, `src/shell_integration.rs:941-961`, `src/update_check.rs:788-805`, `src/workspace.rs:240-260`, `src/session.rs:120-155` (replace unguarded `PermissionsExt`/`OpenOptionsExt`/`DirBuilderExt`/`O_NOFOLLOW`/`symlink`/`flock` with `crate::platform::fs::*`)
- Modify: all production files calling `std::env::var_os("HOME")` → `crate::platform::env::home_dir()`
- Modify: all production files calling `std::fs::canonicalize` / `.canonicalize()` → `crate::platform::fs::canonicalize(...)`

**Interfaces:**
- Consumes: `platform::env::*`, `platform::fs::*`, `platform::time::*`
- Produces: Zero unguarded `localtime_r`, `gethostname`, `clock_gettime`, `PermissionsExt`, `OpenOptionsExt`, `DirBuilderExt`, `O_NOFOLLOW`, `HOME`, or raw `canonicalize` in production modules.

- [ ] **Step 1: Wire `src/paths.rs` to `crate::platform::env`**

Update `src/paths.rs` so non-test `config_dir()`, `state_dir()`, and `croft_cache_dir()` call `crate::platform::env::{config_dir, state_dir, cache_dir}` (keeping `#[cfg(test)]` temp-dir behavior intact).

- [ ] **Step 2: Replace all `localtime_r`, `gethostname`, and `clock_gettime` call sites**

- `src/snippets.rs:288-315`: call `crate::platform::time::local_datetime(secs as i64)?`, map `dt.year`, `dt.month`, `dt.day`, `dt.weekday`, `dt.hour`, `dt.minute`, `dt.second`, `dt.utc_offset_mins`.
- `src/widgets/codeql.rs:436-443`: call `crate::platform::time::local_datetime(started as i64)` → `format!("{:02}:{:02}", dt.hour, dt.minute)`.
- `src/widgets/terminal/clock.rs:164-170`: call `crate::platform::time::local_datetime((millis / 1000) as i64)` → `format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second)`.
- `src/widgets/file_tree/android_trash.rs`: replace `localtime_r` with `crate::platform::time::local_datetime` and gate `libc::EXDEV` under `#[cfg(unix)]` (or gate Android trash helpers under `#[cfg(target_os = "android")]` / `#[cfg(unix)]` where appropriate).
- `src/command_history.rs:44-55` and `src/session_host/identity.rs:49-64`: delegate directly to `crate::platform::env::hostname()`.
- `src/startup_trace.rs:185-193`: delegate `thread_cpu()` to `crate::platform::time::thread_cpu_time()`.

- [ ] **Step 3: Replace unguarded Unix fs permissions, `O_NOFOLLOW`, `symlink`, and `flock`**

Update each file listed above:
- `is_executable_file` (`src/app/debugger/adapters.rs:77`): delegate to `crate::platform::fs::is_executable(path)`.
- Private file/dir creation (`src/app/runners.rs`, `src/app/save_restore.rs`, `src/app/terminal_panes.rs`, `src/config_layers.rs`, `src/history.rs`, `src/hot_exit.rs`, `src/live_run.rs`, `src/update_check.rs`, `src/workspace.rs`): call `crate::platform::fs::set_private_file_mode(&mut opts)`, `set_private_dir_mode(&mut builder)`, and `open_nofollow(&mut opts)`.
- `PermissionsExt::from_mode` writes (`src/app/code_scanning.rs`, `src/app/codeql/{contextual,run}.rs`, `src/codeql_cli.rs`, `src/notebook.rs`): gate mode-setting blocks with `#[cfg(unix)]`.
- `src/shell_integration.rs:941-961` (`ensure_pane_bin`): replace `OsStrExt::as_bytes()` with `exe.as_os_str().as_encoded_bytes()` and `std::os::unix::fs::symlink` with `crate::platform::fs::create_file_link(exe, &tmp)` (and on Windows name the link `croft.exe`).
- `src/session.rs:123-152` (`acquire_pair_host_lock`): use `crate::platform::fs::try_lock_exclusive(&file)`.

- [ ] **Step 4: Replace direct `HOME` env reads and production `canonicalize` calls**

- Update production calls to `std::env::var_os("HOME")` to call `crate::platform::env::home_dir()`.
- Update production calls to `std::fs::canonicalize(p)` / `p.canonicalize()` to call `crate::platform::fs::canonicalize(p)`.
- Verify with PowerShell audit: zero unguarded `localtime_r`, `gethostname`, `clock_gettime`, `O_NOFOLLOW`, `PermissionsExt`, `DirBuilderExt`, `OpenOptionsExt` outside `#[cfg(unix)]` in production files.

- [ ] **Step 5: Commit**

```powershell
git add -u
git commit -m "refactor: route paths, time, hostname, canonicalize, and fs helpers through platform layer"
```

---

### Task 6: Migrate Process Control (`setsid`, `kill`, `killpg`, `SIGPIPE`, `RLIMIT_NOFILE`) Across Production Modules

**Files:**
- Modify: `src/git.rs:285-296`
- Modify: `src/lsp/install.rs:1568-1585`
- Modify: `src/mcp/transport.rs:126-155`
- Modify: `src/dap/transport.rs:165-195`, `248-285`, `368-436`
- Modify: `src/live_run.rs:390-445`
- Modify: `src/testing/worker/control.rs:1-56`
- Modify: `src/hot_exit.rs:250-265`
- Modify: `src/app/mod.rs:5940-5960`, `6418-6448`
- Modify: `src/app/host_integration.rs:1655-1770`
- Modify: `src/app/editor_tabs.rs:1967-1988`
- Modify: `src/cli/run.rs:530-537`
- Modify: `src/port_detect.rs:365-420`
- Modify: `src/voice.rs:205-265`

**Interfaces:**
- Consumes: `platform::process::{detach_from_tty, ManagedChild, is_pid_alive, terminate_pid, raise_fd_limit, reset_sigpipe_default}`, `platform::os::reveal_in_file_manager`
- Produces: All cross-platform subprocess spawning, cancellation, PID liveness checks, and self-relaunch compile cleanly on Windows.

- [ ] **Step 1: Replace `setsid` / `detach_from_tty` across `git`, `lsp`, `mcp`, `dap`**

- `src/git.rs::detach_from_tty`: call `crate::platform::process::detach_from_tty(cmd)`.
- `src/lsp/install.rs::detached_shell`: call `crate::platform::process::detach_from_tty(&mut cmd)`.
- `src/mcp/transport.rs`: replace `CommandExt::pre_exec(setsid)` with `crate::platform::process::detach_from_tty(&mut cmd)`.
- `src/dap/transport.rs`: replace both `setsid` blocks with `crate::platform::process::detach_from_tty(&mut cmd)`; in `kill_tree` use `#[cfg(unix)] libc::killpg` + `child.kill()` + `child.wait()` (or store `ManagedChild` in `DapTransport` so Windows kills the Job Object cleanly); in `is_running(pid)` delegate to `crate::platform::process::is_pid_alive(pid as u32)` (keeping Linux `/proc/{pid}/stat` zombie check under `#[cfg(target_os = "linux")]`); in `finish_teardown` replace `libc::kill(pid, SIGKILL)` with `crate::platform::process::terminate_pid(pid as u32)`.

- [ ] **Step 2: Replace process-group kill & PID liveness in `live_run`, `testing`, `hot_exit`, `app`, `cli`, `port_detect`, `voice`**

- `src/live_run.rs`: spawn via `crate::platform::process::ManagedChild::spawn_in_group(&mut cmd)` and tear down via `managed.kill_tree()`.
- `src/testing/worker/control.rs`: store a cross-platform cancel handle (`#[cfg(unix)] pgid: Option<i32>`, `#[cfg(windows)]` terminate via `crate::platform::process::terminate_pid(pid)` + `ManagedChild` Job Object) and spawn via `detach_from_tty` / `process_group(0)`.
- `src/hot_exit.rs::is_gone`, `src/app/mod.rs::process_is_alive`, `src/app/host_integration.rs` lines 1661 & 1762: delegate to `crate::platform::process::is_pid_alive(pid)`.
- `src/cli/run.rs:536`: call `crate::platform::process::reset_sigpipe_default()`.
- `src/app/mod.rs:6418-6448` (`pending_reexec`): keep `CommandExt::exec` under `#[cfg(unix)]`; under `#[cfg(windows)]`, call `if let Err(err) = cmd.spawn() { eprintln!("croft self-relaunch failed: {err}"); } return Ok(());`.
- `src/app/editor_tabs.rs:1967-1988` (`reveal_in_finder`): delegate to `crate::platform::os::reveal_in_file_manager(&path)`.
- `src/port_detect.rs`: keep output scraping (`PortSniffer`, `url_at`, `loopback_url_port`) unconditional; gate `PROBE_FD_LIMIT`, `probe_fd_limit`, and the `lsof`/`ss` socket-polling runner to `#[cfg(unix)]` with a `#[cfg(not(unix))]` stub returning `Vec::new()`.
- `src/voice.rs`: gate `libc::kill` (line 212) and `pre_exec(setsid)` (line 257) under `#[cfg(unix)]` (`voice` is Android/Termux-only).

- [ ] **Step 3: Commit**

```powershell
git add -u
git commit -m "refactor(process): route setsid, kill, PID liveness, and relaunch through platform layer"
```

---

### Task 7: Module-Boundary Gating for Tier-3 Unix-Only Subsystems & Temporary IPC Stubs

**Files:**
- Modify: `src/session.rs`, `src/session_cli.rs`, `src/session_host.rs` (+ submodules), `src/session_record.rs`, `src/pane_host.rs`
- Modify: `src/collab.rs`, `src/collab_agent.rs`, `src/pair/seat.rs`, `src/pair_host.rs`
- Modify: `src/web/mod.rs`, `src/web/tls.rs`
- Modify: `src/remote.rs`, `src/remote/{control,cross_build,session}.rs`, `src/remote_prebuilt.rs`, `src/remote_bulk.rs`, `src/remote_connect.rs`, `src/config_sync.rs`
- Modify: `src/devcontainer.rs`, `src/gui_path.rs`, `src/cli/links.rs`
- Modify: `src/view_ipc.rs`, `src/agent_approval.rs`, `src/agent_hook.rs`
- Modify: `src/app/mod.rs:2265-2270`, `src/app/host_integration.rs:85-110`, `990-1060`, `1570-1585`

**Interfaces:**
- Consumes: `platform::ipc::{LocalListener, LocalStream}`, `platform::process::raise_fd_limit`
- Produces: Every Tier-3 and IPC module compiles cleanly on Windows with zero `std::os::unix` imports outside `#[cfg(unix)]`, while all CLI subcommands stay registered in `CliCommand` and return clear Windows unsupported messages.

- [ ] **Step 1: Gate Tier-3 Unix-only modules at module boundary**

For each Tier-3 module below, gate its Unix implementation with `#[cfg(unix)]` and add a concise `#[cfg(not(unix))]` module/block exporting the exact same public symbols used by `cli/run.rs` and `app/`:
- `src/session_host.rs`: gate submodules (`attach`, `connection`, `frame_write`, `inner_channel`, `launch`, `outbox`, `pty`, `server`, `swap`) behind `#[cfg(unix)]`; provide `#[cfg(not(unix))]` stubs for `serve`, `attach_or_create`, `InnerChannel` (inert), `Control`/`Frame` types if referenced elsewhere.
- `src/session_cli.rs`: `attach()` and `list()` return `anyhow::bail!("croft attach/ls is not supported on native Windows; run croft in WSL2 for persistent sessions")`.
- `src/session_record.rs`: `record()` returns `anyhow::bail!("croft record is not supported on native Windows; run croft in WSL2 for persistent sessions")`.
- `src/pane_host.rs`: gate Unix `serve`/`spawn`/`PaneClient` under `#[cfg(unix)]`; `#[cfg(not(unix))]` `serve()` returns `anyhow::bail!("pane-host is not supported on native Windows")`.
- `src/session.rs`: gate `bind_socket_0600` under `#[cfg(unix)]`.
- `src/collab.rs`, `src/collab_agent.rs`, `src/pair/seat.rs`, `src/pair_host.rs`: keep pure protocol/data structs cross-platform; gate Unix socket / relay / `setsid` runtime behind `#[cfg(unix)]` with inert `#[cfg(not(unix))]` stubs (`ensure_relay` / `relay_serve` / `collab_agent::run` return actionable Windows errors; `CollabChannel::connect` returns `None`).
- `src/web/mod.rs` & `src/web/tls.rs`: gate Unix socket bridge/TLS under `#[cfg(unix)]`; `#[cfg(not(unix))] pub fn run(...)` returns `anyhow::bail!("croft web is not supported on native Windows; run croft in WSL2")`.
- `src/remote.rs` (+ `remote/{control,cross_build,session}.rs`, `remote_prebuilt.rs`): delegate `raise_fd_limit()` to `crate::platform::process::raise_fd_limit()`; gate SSH `ControlMaster` / `tcflush` / `PermissionsExt` implementations under `#[cfg(unix)]`; `#[cfg(not(unix))]` `launch_croft`, `launch_croft_with`, `launch_only` return `anyhow::bail!("croft remote is not supported on native Windows (Windows OpenSSH lacks ControlMaster); run croft in WSL2")`.
- `src/devcontainer.rs`: keep pure JSON config parser cross-platform; gate container launch under `#[cfg(unix)]` with `#[cfg(not(unix))]` returning `anyhow::bail!("croft devcontainer is not supported on native Windows; run croft in WSL2")`.
- `src/gui_path.rs`: gate Unix login-shell probe under `#[cfg(unix)]`; `#[cfg(not(unix))]` `repair()` and `remember_login_path()` are no-ops.
- `src/cli/links.rs`: gate `install-link-handler` / `open-link` Unix exec under `#[cfg(unix)]` with clear Windows unsupported error under `#[cfg(not(unix))]`.

- [ ] **Step 2: Switch `view_ipc`, `agent_approval`, `agent_hook`, and `App` listeners to `crate::platform::ipc`**

- In `src/app/mod.rs:2265-2270`, change field types to `Option<crate::platform::ipc::LocalListener>`.
- In `src/app/host_integration.rs`, use `crate::platform::ipc::{LocalListener, LocalStream}`; under `#[cfg(not(unix))]`, `bind_view_socket()` returns `None` quietly (no `VIEW_BIND_ERROR`) and `drain_view_requests()` / `drain_hook_requests()` are no-ops.
- In `src/view_ipc.rs`: gate Unix `OsStrExt` / `UnixStream` / `MetadataExt` functions under `#[cfg(unix)]`; under `#[cfg(not(unix))]`, `run()` and `edit()` return `anyhow::bail!("croft view/edit is not yet supported on native Windows (planned in Sub-project 5)")`.
- In `src/agent_approval.rs` & `src/agent_hook.rs`: gate `UnixListener`/`UnixStream` code under `#[cfg(unix)]`; under `#[cfg(not(unix))]`, `agent_approval::bind()` returns `Err(io::Error::new(io::ErrorKind::Unsupported, "Windows IPC not yet implemented"))` (and `App::new` suppresses `Unsupported` into `hook_bind_error = None`), `accept_into()` returns `false`, and `agent_hook::connect_croft()` returns `None`.

- [ ] **Step 3: Commit**

```powershell
git add -u
git commit -m "feat(windows): gate Tier-3 unix-only subsystems and wire platform::ipc boundary"
```

---

### Task 8: Minimal Working ConPTY Terminal Backend (`src/widgets/terminal/`)

**Files:**
- Modify: `src/widgets/terminal.rs:607-616`
- Modify: `src/widgets/terminal/backend.rs:66-432`
- Modify: `src/widgets/terminal/spawn.rs:83-105`, `159-175`, `258-405`
- Modify: `src/widgets/terminal/processes.rs:58-165`, `325-355`

**Interfaces:**
- Consumes: `platform::env::default_shell()`, `portable_pty::native_pty_system()`
- Produces: Local ConPTY terminal pane that spawns PowerShell (`pwsh.exe`/`powershell.exe`/`cmd.exe`), streams output into `alacritty_terminal` via a channel reader thread with `recv_timeout` deadlines, resizes cleanly, and tears down without hanging on `Drop`.

- [ ] **Step 1: Gate Unix-only `PaneLink` / `PaneBackend::Hosted` and add Windows channel reader in `backend.rs` & `terminal.rs`**

In `src/widgets/terminal.rs:609-615`:
```rust
struct PaneLink {
    writer: Box<dyn Write + Send>,
    #[cfg(unix)]
    reader: Box<dyn Read + Send>,
    #[cfg(unix)]
    poll_fd: std::os::fd::RawFd,
    #[cfg(windows)]
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    shell_pid: Option<i32>,
    backend: PaneBackend,
}
```

In `src/widgets/terminal/backend.rs`:
1. Gate `PaneSource::Attach`, `PaneBackend::Hosted`, `HostInput`, `PaneLink::attach`, `wait_pty_readable`, `poll_pty`, and `PtyWait` with `#[cfg(unix)]`.
2. In `PaneBackend::process_group_leader()`: under `#[cfg(windows)]`, return `None`.
3. In `PaneLink::local`:
   - Under `#[cfg(unix)]`: keep existing implementation untouched.
   - Under `#[cfg(windows)]`:
```rust
#[cfg(windows)]
pub(super) fn local(cmd: CommandBuilder, cols: u16, rows: u16) -> Result<Self> {
    let pair = native_pty_system()
        .openpty(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty")?;
    let child = pair.slave.spawn_command(cmd).context("spawn child")?;
    let shell_pid = child.process_id().map(|p| p as i32);
    drop(pair.slave);
    let writer = pair.master.take_writer().context("take writer")?;
    let mut reader = pair.master.try_clone_reader().context("clone reader")?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(Self {
        writer,
        rx,
        shell_pid,
        backend: PaneBackend::Local {
            master: pair.master,
            child,
        },
    })
}
```
4. In `Drop for PtyTerminal` (`backend.rs:408-432`):
   - Under `#[cfg(unix)]`: keep existing `backend.end()`, rewind release, `shutdown_w` drop, and `handle.join()`.
   - Under `#[cfg(windows)]`: call `self.backend.end()` (kills child and drops `MasterPty` so ConPTY closes the output pipe and unblocks the reader thread), release rewind budget, and join `reader_thread` only if finished or with a short bounded poll (up to 250 ms) so a grandchild holding a ConPTY handle can never freeze UI exit.

- [ ] **Step 2: Update `src/widgets/terminal/spawn.rs` and `processes.rs`**

- In `spawn.rs:85` and `98`: replace `std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())` with `crate::platform::env::default_shell()`.
- In `spawn.rs::spawn_from`:
  - Gate `shutdown_r`/`shutdown_w` (`std::io::pipe()`) and the `poll_pty`/`wait_pty_readable`/`reader.read` read branch under `#[cfg(unix)]`.
  - Under `#[cfg(windows)]`, read chunks from `rx` using `rx.recv_timeout(...)`:
    - When `port_sniffer.has_pending()` is true, wait `Duration::from_millis(PORT_URL_SETTLE_MS as u64)`; on `RecvTimeoutError::Timeout`, flush `port_sniffer` and continue; on `Disconnected`, break.
    - Else if `let Some(deadline) = watch_engine.deadline()`, wait `deadline.saturating_duration_since(Instant::now())`; on `Timeout`, expire `watch_engine` and continue.
    - Else call `rx.recv()`; on `Err(_)`, break.
    - Feed the received `chunk: Vec<u8>` into the exact same downstream sniffer/parser pipeline.
- In `src/widgets/terminal/processes.rs`: ensure `cwd_of_pid` and `foreground_group_of` have `#[cfg(not(any(target_os = "linux", target_os = "macos")))]` arms returning `None`.

- [ ] **Step 3: Commit**

```powershell
git add src/widgets/terminal.rs src/widgets/terminal/backend.rs src/widgets/terminal/spawn.rs src/widgets/terminal/processes.rs
git commit -m "feat(terminal): add Windows ConPTY channel reader and PowerShell default shell"
```

---

### Task 9: Final Compile Sweep & Clean Windows Binary Build

**Files:**
- Modify: any remaining files reported by `cargo check --bin croft` (expected small cleanup in `src/widgets/editor/{disk,viewers}.rs`, `src/app/bottom_panel.rs`, `src/app/mod.rs`, `src/lsp/manager/discovery.rs`, `src/widgets/file_tree/{move_and_copy,new_and_rename}.rs`).

**Interfaces:**
- Consumes: All changes from Tasks 1–8
- Produces: Clean `cargo check --bin croft` and `cargo build --bin croft` producing `target/debug/croft.exe`.

- [ ] **Step 1: Run `cargo check --bin croft` and fix every remaining compile error**

Run: `cargo check --bin croft`
Inspect all compiler errors in one pass, fix every reported file cleanly using `crate::platform::*` or `#[cfg(unix)]` (never adding `unsafe` Win32 FFI outside `src/platform/` or `src/clipboard.rs`), and re-run `cargo check --bin croft` until it exits 0.

- [ ] **Step 2: Build the Windows binary**

Run: `cargo build --bin croft`
Expected: Exits 0 and creates `target\debug\croft.exe`.

- [ ] **Step 3: Commit**

```powershell
git add -u
git commit -m "fix(windows): resolve remaining compile errors for clean x86_64-pc-windows-msvc build"
```

---

### Task 10: Automated & CLI Verification

**Files:**
- None (verification task; fix any defects discovered before completing).

**Interfaces:**
- Consumes: `target\debug\croft.exe` and `tests\platform.rs`
- Produces: Verified passing test suite and CLI smoke test evidence.

- [ ] **Step 1: Run the platform unit test suite**

Run: `cargo test --test platform`
Expected: All tests pass (0 failed).

- [ ] **Step 2: Run CLI & Tier-3 error smoke checks**

Run:
```powershell
.\target\debug\croft.exe --version
.\target\debug\croft.exe --build-info
.\target\debug\croft.exe attach; "attach exit: $LASTEXITCODE"
.\target\debug\croft.exe remote example; "remote exit: $LASTEXITCODE"
.\target\debug\croft.exe web; "web exit: $LASTEXITCODE"
.\target\debug\croft.exe view README.md; "view exit: $LASTEXITCODE"
```
Expected:
- `--version` prints `croft 0.2.159`
- `--build-info` prints commit hash + valid UTC ISO-8601 timestamp (NOT `unknown`!)
- `attach`, `remote`, `web`, `view` each print a clear Windows unsupported message and exit non-zero without panicking.

- [ ] **Step 3: Interactive smoke test checklist (for the user in Windows Terminal & WezTerm)**

Present the ready-to-run verification checklist:
1. Run `.\target\debug\croft.exe .` inside PowerShell in **Windows Terminal** and **WezTerm**.
2. Confirm Explorer loads the workspace without `\\?\` path prefixes.
3. Open a file, edit, save, and verify contents on disk.
4. Test `Ctrl+C` / `Ctrl+V` clipboard round-trip with another Windows app.
5. Test Search (`Ctrl+Shift+F`) and Source Control (Git status).
6. Focus the bottom PowerShell ConPTY pane, run `Get-Location` and `echo hello`.
7. Quit with `Ctrl+Q` and confirm croft exits immediately without hanging or leaving orphan PowerShell processes.
