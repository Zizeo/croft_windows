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
