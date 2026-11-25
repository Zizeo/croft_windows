#![allow(dead_code)]

#[path = "../src/platform/mod.rs"]
mod platform;
#[path = "../src/program_status.rs"]
mod program_status;
#[path = "../src/shell_integration.rs"]
mod shell_integration;

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
        .truncate(false)
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
#[cfg_attr(not(windows), allow(clippy::drop_non_drop))]
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

#[cfg(windows)]
#[test]
fn powershell_shim_emits_osc133_and_osc7_in_live_powershell() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_dir = tmp.path().join("cfg");
    let pane_bin = tmp.path().join("pane_bin");
    let work_dir = tmp.path().join("work dir");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&pane_bin).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();

    let shim = shell_integration::ensure_powershell_shim(&cfg_dir).unwrap();
    assert!(shim.is_file());

    let mut shells: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("powershell.exe")];
    if let Some(pwsh) = platform::env::resolve_on_path("pwsh.exe") {
        shells.push(pwsh);
    }

    let script = format!(
        ". '{}'; Write-Output \"PATH_FIRST=$($env:PATH.Split([IO.Path]::PathSeparator)[0])\"; prompt; $Global:__CroftCommandInFlight = $true; cmd /c 'exit 7'; prompt",
        shim.display().to_string().replace('\'', "''")
    );

    for shell in shells {
        let out = std::process::Command::new(&shell)
            .args([
                "-NoProfile",
                "-NoLogo",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .current_dir(&work_dir)
            .env(shell_integration::PANE_BIN_ENV, &pane_bin)
            .output()
            .unwrap_or_else(|e| panic!("failed to run {}: {e}", shell.display()));

        assert!(
            out.status.success(),
            "{} exited with {:?}: stderr={}",
            shell.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout_lossy = String::from_utf8_lossy(&out.stdout);
        let expected_first = format!("PATH_FIRST={}", pane_bin.display());
        assert!(
            stdout_lossy.contains(&expected_first),
            "{} did not prepend CROFT_PANE_BIN to PATH: stdout={stdout_lossy:?}",
            shell.display()
        );

        let events: Vec<_> = shell_integration::OscSniffer::default()
            .scan(&out.stdout)
            .into_iter()
            .map(|(_, ev)| ev)
            .collect();
        assert!(
            events.contains(&shell_integration::OscEvent::PromptStart),
            "{} missing OSC 133;A in {events:?}",
            shell.display()
        );
        assert!(
            events.contains(&shell_integration::OscEvent::PromptEnd),
            "{} missing OSC 133;B in {events:?}",
            shell.display()
        );
        assert!(
            events.contains(&shell_integration::OscEvent::CommandEnd(Some(7))),
            "{} missing OSC 133;D;7 in {events:?}",
            shell.display()
        );
        let cwd_event = events.iter().find_map(|e| match e {
            shell_integration::OscEvent::Cwd(p, Some(h)) if !h.is_empty() => Some(p),
            _ => None,
        });
        let cwd_path = cwd_event.unwrap_or_else(|| {
            panic!(
                "{} missing OSC 7 CWD+host event in {events:?}",
                shell.display()
            )
        });
        assert!(
            cwd_path.is_dir(),
            "{} OSC 7 path {:?} is not an existing directory",
            shell.display(),
            cwd_path
        );
    }
}

#[test]
fn terminal_probe_parser_decodes_kitty_xtwinops_16t_14t_and_da1() {
    use platform::terminal::{
        HostProbeResult, has_complete_da1, parse_host_probe_bytes, parse_host_probe_bytes_with_grid,
    };

    let combined = b"\x1b[?0u\x1b[6;20;10t\x1b[4;600;1200t\x1b[?65;4;6;18;22;52c";
    assert!(has_complete_da1(combined));
    assert_eq!(
        parse_host_probe_bytes(combined),
        HostProbeResult {
            sixel: true,
            kitty_keyboard: true,
            answered: true,
            cell_pixel_size: Some((10, 20)),
            window_pixel_size: Some((1200, 600)),
        }
    );

    // Windows Terminal without Sixel advertises attributes like 14 and 42;
    // make sure neither matches Sixel attribute 4 as a substring.
    let wt_no_sixel = b"\x1b[?1u\x1b[?61;6;7;14;21;22;23;24;28;32;42c";
    assert!(has_complete_da1(wt_no_sixel));
    assert_eq!(
        parse_host_probe_bytes(wt_no_sixel),
        HostProbeResult {
            sixel: false,
            kitty_keyboard: true,
            answered: true,
            cell_pixel_size: None,
            window_pixel_size: None,
        }
    );

    // CSI 14 t fallback computes cell size from grid when CSI 16 t is absent.
    let only_14t = b"\x1b[4;600;1200t\x1b[?61;4c";
    assert_eq!(
        parse_host_probe_bytes_with_grid(only_14t, Some((120, 30))),
        HostProbeResult {
            sixel: true,
            kitty_keyboard: false,
            answered: true,
            cell_pixel_size: Some((10, 20)),
            window_pixel_size: Some((1200, 600)),
        }
    );

    // Partial / zero-valued sequences are ignored safely.
    assert!(!has_complete_da1(b"\x1b[?62;4"));
    assert_eq!(
        parse_host_probe_bytes(b"\x1b[?62;4\x1b[6;0;10t"),
        HostProbeResult::default()
    );
}

#[test]
fn terminal_probe_safe_on_non_console_handles() {
    let t0 = std::time::Instant::now();
    let res = platform::terminal::ask_host_terminal().answer();
    assert!(
        t0.elapsed() < std::time::Duration::from_millis(500),
        "terminal probe blocked unexpectedly: {:?}",
        t0.elapsed()
    );
    if let Some((w, h)) = res.cell_pixel_size {
        assert!(w > 0 && h > 0, "invalid probed cell pixel size: ({w}, {h})");
    }
    if let Some((w, h)) = platform::terminal::console_font_cell_pixel() {
        assert!(w > 0 && h > 0, "invalid font cell pixel size: ({w}, {h})");
    }
    #[cfg(windows)]
    if let Some((cols, rows)) = platform::terminal::console_grid_size() {
        assert!(cols > 0 && rows > 0, "invalid grid size: ({cols}, {rows})");
    }
}

#[test]
fn toolchain_venv_python_dlv_lldb_and_node_modules_tsc_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // 1. Virtual-environment interpreter path & discovery.
    let expected_debug_py = if cfg!(windows) {
        root.join("debug-venv").join("Scripts").join("python.exe")
    } else {
        root.join("debug-venv").join("bin").join("python")
    };
    assert_eq!(
        platform::env::venv_python_path(&root.join("debug-venv")),
        expected_debug_py
    );

    let win_venv = root.join("win-venv");
    let win_py = win_venv.join("Scripts").join("python.exe");
    std::fs::create_dir_all(win_py.parent().unwrap()).unwrap();
    std::fs::write(&win_py, "").unwrap();
    assert_eq!(
        platform::env::venv_python_in(&win_venv),
        Some(win_py.clone())
    );

    let posix_venv = root.join("posix-venv");
    let posix_py = posix_venv.join("bin").join("python");
    std::fs::create_dir_all(posix_py.parent().unwrap()).unwrap();
    std::fs::write(&posix_py, "").unwrap();
    assert_eq!(
        platform::env::venv_python_in(&posix_venv),
        Some(posix_py.clone())
    );

    // 2. Delve resolution order & `.exe` handling.
    let dlv_name = if cfg!(windows) { "dlv.exe" } else { "dlv" };
    let mk_dlv = |rel: &str| {
        let dir = root.join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(dlv_name), "").unwrap();
        dir
    };
    let on_path = mk_dlv("dlv-path");
    let gobin = mk_dlv("dlv-gobin");
    let home = root.join("home");
    let home_go_bin = mk_dlv("home/go/bin");
    let gopath = root.join("gopath");
    let gopath_bin = mk_dlv("gopath/bin");
    let croft_go = mk_dlv("home/.croft/servers/go");
    let path_env = std::env::join_paths([&on_path]).unwrap();

    assert_eq!(
        platform::env::find_dlv_in(Some(&path_env), Some(&gobin), Some(&gopath), Some(&home)),
        Some(on_path.join(dlv_name))
    );
    assert_eq!(
        platform::env::find_dlv_in(None, Some(&gobin), Some(&gopath), Some(&home)),
        Some(gobin.join(dlv_name))
    );
    assert_eq!(
        platform::env::find_dlv_in(None, None, Some(&gopath), Some(&home)),
        Some(gopath_bin.join(dlv_name))
    );
    assert_eq!(
        platform::env::find_dlv_in(None, None, None, Some(&home)),
        Some(home_go_bin.join(dlv_name))
    );
    std::fs::remove_file(home_go_bin.join(dlv_name)).unwrap();
    assert_eq!(
        platform::env::find_dlv_in(None, None, None, Some(&home)),
        Some(croft_go.join(dlv_name))
    );

    // 3. `node_modules/.bin/tsc` resolves `.cmd` ahead of POSIX shim on Windows.
    let node_bin = root.join("node_modules").join(".bin");
    std::fs::create_dir_all(&node_bin).unwrap();
    let posix_tsc = node_bin.join("tsc");
    std::fs::write(&posix_tsc, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&posix_tsc, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cmd_tsc = node_bin.join("tsc.cmd");
    std::fs::write(&cmd_tsc, "@echo off\r\n").unwrap();
    let expected_tsc = if cfg!(windows) { cmd_tsc } else { posix_tsc };
    assert_eq!(
        platform::env::executable_in_dir(&node_bin, "tsc"),
        Some(expected_tsc)
    );

    // 4. `lldb-dap` ranking and PATH lookup.
    assert!(
        platform::env::lldb_dap_rank("lldb-dap.exe").unwrap()
            > platform::env::lldb_dap_rank("lldb-vscode.exe").unwrap()
    );
    assert!(
        platform::env::lldb_dap_rank("lldb-vscode.exe").unwrap()
            > platform::env::lldb_dap_rank("lldb-dap-19.exe").unwrap()
    );
    assert!(
        platform::env::lldb_dap_rank("lldb-dap-19.exe").unwrap()
            > platform::env::lldb_dap_rank("lldb-dap-18.exe").unwrap()
    );

    let lldb_old = root.join("lldb-old");
    let lldb_new = root.join("lldb-new");
    std::fs::create_dir_all(&lldb_old).unwrap();
    std::fs::create_dir_all(&lldb_new).unwrap();
    let old_bin = lldb_old.join(if cfg!(windows) {
        "lldb-dap-18.exe"
    } else {
        "lldb-dap-18"
    });
    let new_bin = lldb_new.join(if cfg!(windows) {
        "lldb-dap.exe"
    } else {
        "lldb-dap"
    });
    std::fs::write(&old_bin, "").unwrap();
    std::fs::write(&new_bin, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&old_bin, &new_bin] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let lldb_path = std::env::join_paths([&lldb_old, &lldb_new]).unwrap();
    assert_eq!(
        platform::env::find_lldb_dap_on_path(Some(&lldb_path)),
        Some(new_bin)
    );
}

#[test]
fn fs_rename_and_write_retrying_recover_from_sharing_violation_and_refuse_readonly() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("note.txt");
    let staged = tmp.path().join("note.tmp");

    platform::fs::write_retrying(&target, b"initial").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"initial");

    std::fs::write(&staged, b"renamed").unwrap();
    platform::fs::rename_retrying(&staged, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"renamed");

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // Hold an exclusive (share_mode = 0) handle briefly; write_retrying recovers.
        let (tx, rx) = std::sync::mpsc::channel();
        let lock_path = target.clone();
        let holder = std::thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&lock_path)
                .unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(25));
            drop(file);
        });
        rx.recv().unwrap();
        platform::fs::write_retrying(&target, b"after-write-lock").unwrap();
        holder.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"after-write-lock");

        // Hold an exclusive handle briefly; rename_retrying recovers.
        std::fs::write(&staged, b"after-rename-lock").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let lock_path = target.clone();
        let holder = std::thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&lock_path)
                .unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(25));
            drop(file);
        });
        rx.recv().unwrap();
        platform::fs::rename_retrying(&staged, &target).unwrap();
        holder.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"after-rename-lock");

        // Read-only files fail immediately without burning the 305 ms retry budget.
        let ro = tmp.path().join("readonly.txt");
        std::fs::write(&ro, b"immutable").unwrap();
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms.clone()).unwrap();

        std::fs::write(&staged, b"attempt").unwrap();
        let t0 = std::time::Instant::now();
        assert!(platform::fs::write_retrying(&ro, b"attempt").is_err());
        assert!(platform::fs::rename_retrying(&staged, &ro).is_err());
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(120),
            "read-only refusal unexpectedly slept: {:?}",
            t0.elapsed()
        );

        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&ro, perms).unwrap();
    }
}

#[test]
fn ipc_local_listener_nonblocking_roundtrip_auth_and_cleanup() {
    use std::io::{BufRead, BufReader, ErrorKind, Write};

    let tmp = tempfile::tempdir().unwrap();
    let sock_path = tmp.path().join("view-test.sock");

    let listener = platform::ipc::LocalListener::bind(&sock_path).unwrap();
    listener.set_nonblocking(true).unwrap();

    // Non-blocking listener reports WouldBlock when idle.
    let err = listener.accept().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WouldBlock);

    // Concurrent bind while live returns AddrInUse and does not leave a broken
    // probe connection in `listener`'s accept queue.
    #[cfg(windows)]
    {
        let dup = platform::ipc::LocalListener::bind(&sock_path).unwrap_err();
        assert_eq!(dup.kind(), ErrorKind::AddrInUse);
        let idle = listener.accept().unwrap_err();
        assert_eq!(idle.kind(), ErrorKind::WouldBlock);
    }

    // Valid client connects and sends a pipelined request before accept() runs.
    let mut client = platform::ipc::LocalStream::connect(&sock_path).unwrap();
    client.write_all(b"ping\n").unwrap();
    client.flush().unwrap();

    let (mut server, _) = listener.accept().unwrap();
    server.set_nonblocking(false).unwrap();
    let mut server_reader = BufReader::new(&server);
    let mut line = String::new();
    server_reader.read_line(&mut line).unwrap();
    assert_eq!(line, "ping\n");

    server.write_all(b"pong\n").unwrap();
    server.flush().unwrap();
    let mut client_reader = BufReader::new(&client);
    let mut reply = String::new();
    client_reader.read_line(&mut reply).unwrap();
    assert_eq!(reply, "pong\n");

    // Unauthorized loopback connection with wrong capability token is rejected.
    #[cfg(windows)]
    {
        let mut rogue = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        rogue.write_all(b"wrong-token\n").unwrap();
        rogue.flush().unwrap();
        let auth_err = listener.accept().unwrap_err();
        assert_eq!(auth_err.kind(), ErrorKind::PermissionDenied);
    }

    drop(listener);
    #[cfg(windows)]
    {
        assert!(!sock_path.exists(), "descriptor must be removed on drop");
        let rebound = platform::ipc::LocalListener::bind(&sock_path).unwrap();
        drop(rebound);
    }
}

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

#[cfg(windows)]
#[test]
fn ipc_descriptor_nonce_mode_switch_and_flood_expiry() {
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let sock1 = tmp.path().join("n1.sock");
    let sock2 = tmp.path().join("n2.sock");
    let l1 = platform::ipc::LocalListener::bind(&sock1).unwrap();
    let l2 = platform::ipc::LocalListener::bind(&sock2).unwrap();

    // Descriptor format: <port>\n<32 lowercase hex CSPRNG nonce>\n<pid>\n
    let d1 = std::fs::read_to_string(&sock1).unwrap();
    let d2 = std::fs::read_to_string(&sock2).unwrap();
    let lines1: Vec<&str> = d1.lines().collect();
    let lines2: Vec<&str> = d2.lines().collect();
    assert_eq!(lines1.len(), 3, "expected 3-line descriptor, got {d1:?}");
    assert_eq!(lines2.len(), 3, "expected 3-line descriptor, got {d2:?}");
    let nonce1 = lines1[1];
    let nonce2 = lines2[1];
    assert_eq!(nonce1.len(), 32);
    assert_eq!(nonce2.len(), 32);
    assert!(
        nonce1
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert!(
        nonce2
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert_ne!(nonce1, nonce2);
    assert_eq!(lines1[2], std::process::id().to_string());
    drop(l2);

    // Overlong line (> MAX_TOKEN_LINE) is rejected immediately with PermissionDenied.
    l1.set_nonblocking(true).unwrap();
    let mut long_rogue = std::net::TcpStream::connect(l1.local_addr().unwrap()).unwrap();
    long_rogue.write_all(&[b'x'; 70]).unwrap();
    long_rogue.flush().unwrap();
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::PermissionDenied);

    // Partial non-blocking auth survives switching to blocking mode without losing buffered bytes.
    let mut partial_client = std::net::TcpStream::connect(l1.local_addr().unwrap()).unwrap();
    partial_client.write_all(&nonce1.as_bytes()[..16]).unwrap();
    partial_client.flush().unwrap();
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::WouldBlock);

    l1.set_nonblocking(false).unwrap();
    partial_client.write_all(&nonce1.as_bytes()[16..]).unwrap();
    partial_client.write_all(b"\nhello\n").unwrap();
    partial_client.flush().unwrap();
    let (srv, _) = l1.accept().unwrap();
    let mut line = String::new();
    BufReader::new(&srv).read_line(&mut line).unwrap();
    assert_eq!(line, "hello\n");

    // Flood > 32 silent peers, let them expire, then verify new valid client succeeds via pruning.
    l1.set_nonblocking(true).unwrap();
    let addr1 = l1.local_addr().unwrap();
    let _flood: Vec<_> = (0..40)
        .map(|_| std::net::TcpStream::connect(addr1).unwrap())
        .collect();
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::WouldBlock);

    std::thread::sleep(Duration::from_millis(130));
    let mut after_expiry = platform::ipc::LocalStream::connect(&sock1).unwrap();
    after_expiry.write_all(b"after-expiry\n").unwrap();
    after_expiry.flush().unwrap();
    let (srv2, _) = l1
        .accept()
        .expect("expired pending entries must be pruned on full queue");
    srv2.set_nonblocking(false).unwrap();
    let mut line2 = String::new();
    BufReader::new(&srv2).read_line(&mut line2).unwrap();
    assert_eq!(line2, "after-expiry\n");

    // Sub-capacity expired silent peer yields PermissionDenied on next accept.
    let _silent = std::net::TcpStream::connect(addr1).unwrap();
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::PermissionDenied);
    assert_eq!(l1.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}

#[cfg(windows)]
#[test]
fn net_list_tcp_listeners_detects_bound_loopback_socket_and_drop() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let pid = std::process::id();

    let v6_listener = std::net::TcpListener::bind("[::1]:0").ok();
    let v6_port = v6_listener.as_ref().map(|l| l.local_addr().unwrap().port());

    let entries = platform::net::list_tcp_listeners().expect("Windows TCP table available");
    assert!(
        entries
            .iter()
            .any(|e| e.port == port && e.pid == pid && e.is_loopback_or_wildcard),
        "expected IPv4 port {port} owned by pid {pid} in {entries:?}"
    );
    if let Some(v6_port) = v6_port {
        assert!(
            entries
                .iter()
                .any(|e| e.port == v6_port && e.pid == pid && e.is_loopback_or_wildcard),
            "expected IPv6 port {v6_port} owned by pid {pid} in {entries:?}"
        );
    }

    drop(listener);
    drop(v6_listener);
    let after = platform::net::list_tcp_listeners().expect("Windows TCP table available");
    assert!(
        !after.iter().any(|e| e.port == port && e.pid == pid),
        "dropped IPv4 port {port} should no longer appear for pid {pid}"
    );
    if let Some(v6_port) = v6_port {
        assert!(
            !after.iter().any(|e| e.port == v6_port && e.pid == pid),
            "dropped IPv6 port {v6_port} should no longer appear for pid {pid}"
        );
    }
}

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
