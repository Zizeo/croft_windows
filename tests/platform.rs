#![allow(dead_code)]

#[path = "../src/platform/mod.rs"]
mod platform;

#[test]
fn env_home_and_dirs_resolve_non_empty() {
    let home = platform::env::home_dir();
    assert!(home.is_some(), "home_dir() must resolve on this OS");
    assert!(platform::env::config_dir().ends_with("croft"));
    assert!(
        platform::env::state_dir().is_absolute() || platform::env::state_dir().ends_with("croft")
    );
    assert!(
        platform::env::cache_dir().is_absolute() || platform::env::cache_dir().ends_with("croft")
    );
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
        PathBuf::from(r"C:\Users\u\AppData\Local")
            .join("croft")
            .join("state")
    );

    let cache = platform::env::cache_dir_windows(
        Some(OsString::from(r"C:\Users\u\AppData\Local")),
        Some(PathBuf::from(r"C:\Users\u")),
    );
    assert_eq!(
        cache,
        PathBuf::from(r"C:\Users\u\AppData\Local")
            .join("croft")
            .join("cache")
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
fn fs_create_file_link_already_exists_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let existing = dir.path().join("existing.txt");
    std::fs::write(&target, b"new content").unwrap();
    std::fs::write(&existing, b"original content").unwrap();

    let err = platform::fs::create_file_link(&target, &existing).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(&existing).unwrap(),
        "original content"
    );
}

fn spawn_sleep_command() -> std::process::Command {
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
    cmd
}

#[test]
fn process_liveness_and_managed_child_kill_tree() {
    assert!(platform::process::is_pid_alive(std::process::id()));

    let mut cmd = spawn_sleep_command();
    let mut managed = platform::process::ManagedChild::spawn_in_group(&mut cmd).unwrap();
    let pid = managed.id();
    assert!(platform::process::is_pid_alive(pid));
    managed.kill_tree();
    assert!(!platform::process::is_pid_alive(pid));
}

#[test]
fn process_into_child_preserves_liveness() {
    let mut cmd = spawn_sleep_command();
    let managed = platform::process::ManagedChild::spawn_in_group(&mut cmd).unwrap();
    let pid = managed.id();
    let mut child = managed.into_child();

    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        child.try_wait().unwrap().is_none(),
        "into_child() must leave child running"
    );
    assert!(
        platform::process::is_pid_alive(pid),
        "into_child() must not kill child PID {pid}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!platform::process::is_pid_alive(pid));
}

#[test]
fn process_split_killer_tree_killer_and_force_kill_pid() {
    assert!(!platform::process::force_kill_pid(0));

    let mut cmd = spawn_sleep_command();
    let managed = platform::process::ManagedChild::spawn_in_group(&mut cmd).unwrap();
    let pid = managed.id();
    let (mut child, killer) = managed.split_killer();
    let killer_clone = killer.clone();

    assert!(child.try_wait().unwrap().is_none());
    assert!(platform::process::is_pid_alive(pid));

    killer_clone.kill_tree();
    drop(killer_clone);
    let _ = child.wait().unwrap();
    drop(killer);
    assert!(!platform::process::is_pid_alive(pid));

    let mut cmd = spawn_sleep_command();
    let mut child = platform::process::ManagedChild::spawn_in_group(&mut cmd)
        .unwrap()
        .into_child();
    let pid = child.id();
    assert!(platform::process::is_pid_alive(pid));
    assert!(platform::process::force_kill_pid(pid));
    let _ = child.wait().unwrap();
    assert!(!platform::process::is_pid_alive(pid));

    let mut cmd = spawn_sleep_command();
    platform::process::detach_from_tty(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    let pid = child.id();
    let killer = platform::process::tree_killer_for_leader(&child);
    assert!(platform::process::is_pid_alive(pid));
    killer.kill_tree();
    let _ = child.wait().unwrap();
    drop(killer);
    assert!(!platform::process::is_pid_alive(pid));
}

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

#[test]
fn env_executable_in_dir_prefers_windows_extensions_over_posix_shims() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Simulate an npm `.bin` folder containing both a bare POSIX shell script
    // and a Windows `.cmd` wrapper, plus a native `.exe` binary.
    let vtsls_posix = dir.join("vtsls");
    let vtsls_cmd = dir.join("vtsls.cmd");
    let ra_exe = dir.join("rust-analyzer.exe");
    std::fs::write(&vtsls_posix, b"#!/bin/sh\n").unwrap();
    std::fs::write(&vtsls_cmd, b"@echo off\r\n").unwrap();
    std::fs::write(&ra_exe, b"").unwrap();

    #[cfg(windows)]
    {
        assert_eq!(
            platform::env::executable_in_dir(dir, "vtsls"),
            Some(vtsls_cmd)
        );
        assert_eq!(
            platform::env::executable_in_dir(dir, "rust-analyzer"),
            Some(ra_exe)
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&vtsls_posix, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            platform::env::executable_in_dir(dir, "vtsls"),
            Some(vtsls_posix)
        );
    }
}

#[test]
fn lsp_bundled_binary_manifests_include_windows_targets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "assets/extensions/lsp-rust/extension.toml",
        "assets/extensions/lsp-toml/extension.toml",
        "assets/extensions/lsp-cpp/extension.toml",
    ] {
        let content = std::fs::read_to_string(root.join(rel)).unwrap();
        assert!(
            content.contains("\"windows-x86_64\""),
            "{rel} must define a windows-x86_64 target"
        );
    }
    for rel in [
        "assets/extensions/lsp-markdown/extension.toml",
        "assets/catalog/csvlens/extension.toml",
    ] {
        let content = std::fs::read_to_string(root.join(rel)).unwrap();
        let count = content.matches("\"windows-x86_64\"").count();
        assert!(
            count >= 2,
            "{rel} must define windows-x86_64 in both targets and sha256 (found {count})"
        );
    }
}

#[test]
fn remote_and_sync_config_cli_subcommands_are_enabled() {
    let exe = env!("CARGO_BIN_EXE_croft");
    for args in [["remote", "--help"], ["sync-config", "--help"]] {
        let out = std::process::Command::new(exe)
            .args(args)
            .output()
            .expect("invoke croft binary");
        assert!(
            out.status.success(),
            "`croft {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("not supported on native Windows"),
            "`croft {}` unexpectedly reports Windows unsupported",
            args.join(" ")
        );
    }
}
