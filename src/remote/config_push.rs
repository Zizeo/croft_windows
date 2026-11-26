//! Pushing the user's config to the remote: the files that sync, how each is shipped
//! and logged, and `croft sync-config` with its diff.

use super::*;

/// The remote's config-file hashes, or `None` when they can't be read.
pub(super) fn read_remote_hashes(
    mut command: Command,
) -> Option<std::collections::BTreeMap<String, String>> {
    // Waits out "Text file busy": ssh just installed or updated, or a
    // test's stand-in ssh written a moment ago while another test forked.
    let out = crate::review_ops::spawn_retrying_busy(&mut command)
        .and_then(std::process::Child::wait_with_output)
        .ok()?;
    if !out.status.success() {
        return None;
    }
    crate::config_sync::parse_remote_listing(&String::from_utf8_lossy(&out.stdout))
}

/// Push the user's syncable config to the remote (#262), so keybindings,
/// snippets, triggers and matchers follow the binary that already follows
/// them.
///
/// Called from `run_croft_session`'s prologue rather than off the back of
/// the install: when croft is already present the install runs on a
/// detached thread and the session opens without waiting for it, so an
/// install-completion hook would skip the common case entirely.
///
/// Best-effort by construction. A remote whose config cannot be written is
/// a remote that still opens with defaults, which is exactly today's
/// behaviour — failing the connect over it would be a regression. Every
/// outcome is reported so a silent no-op is distinguishable from a silent
/// success.
pub(super) fn push_config_files(
    ssh: &SshControl,
    resolution: &SyncResolution,
    log: &mut dyn FnMut(String),
) {
    // User layers only: a workspace must not decide what leaves the laptop.
    let prefs = crate::config_layers::load_merged(None).prefs;
    if crate::config_sync::host_excluded(&ssh.host, &prefs.config_sync_excluded_hosts) {
        log(format!(
            "Config sync: off for {} (config_sync_excluded_hosts)",
            ssh.host
        ));
        return;
    }
    let (files, skipped) = crate::config_sync::apply_exclusions(
        crate::config_sync::local_files(),
        &prefs.config_sync_excluded_files,
    );
    if !skipped.is_empty() {
        log(format!(
            "Config sync: not pushing {} (config_sync_excluded_files)",
            skipped.join(", ")
        ));
    }
    if files.is_empty() {
        return;
    }
    // `mkdir -p` first: rsync will not create the remote parent, and a
    // fresh box has no ~/.config/croft until croft has run there once.
    // `background_command` for its `-n`: this runs before the session
    // takes the terminal, and a command that inherits stdin would race the
    // shell for the user's keystrokes.
    let mut mk = ssh.background_shell("mkdir -p ~/.config/croft");
    if !matches!(mk.status(), Ok(st) if st.success()) {
        log(String::from(
            "Config sync: could not create ~/.config/croft on the remote; skipping",
        ));
        return;
    }

    // What is there now, so a copy edited on the remote is never replaced
    // (#262). Unreadable means unknown, and unknown is not permission.
    let Some(remote) = read_remote_hashes(ssh_socket_read(
        &ssh.socket_path,
        &ssh.host,
        &crate::config_sync::remote_hash_script(),
    )) else {
        log(String::from(
            "Config sync: could not read the remote's copies; pushing nothing rather than risk overwriting them",
        ));
        return;
    };
    let state_path = crate::config_sync::SyncState::path();
    let mut state = crate::config_sync::SyncState::load(&state_path);

    let bulk = crate::remote_bulk::establish(&ssh.host, &ssh.socket_path, |_| {});
    let mut pushed = Vec::new();
    let mut failed = Vec::new();
    let mut kept = Vec::new();
    let mut conflicts = Vec::new();
    for (syncable, local) in &files {
        let Ok(bytes) = std::fs::read(local) else {
            failed.push(syncable.name);
            continue;
        };
        let local_hash = crate::config_sync::content_hash(&bytes);
        let there = remote.get(syncable.name).map(String::as_str);
        let chosen = |list: &[String]| list.iter().any(|n| n == syncable.name);
        let plan = if chosen(&resolution.take_local) {
            crate::config_sync::Plan::Push
        } else if chosen(&resolution.keep_remote) && there.is_some_and(|r| r != local_hash) {
            let entry = state.file_mut(&ssh.host, syncable.name);
            entry.kept = there.map(|r| (r.to_string(), local_hash.clone()));
            crate::config_sync::Plan::Kept
        } else {
            crate::config_sync::plan(&local_hash, there, state.file(&ssh.host, syncable.name))
        };
        match plan {
            crate::config_sync::Plan::Push => {
                let remote_rel = format!(".config/croft/{}", syncable.name);
                match ship_local_file_to_remote(
                    &bulk.lane,
                    &ssh.socket_path,
                    &ssh.host,
                    local,
                    &remote_rel,
                ) {
                    Ok(()) => {
                        let entry = state.file_mut(&ssh.host, syncable.name);
                        entry.pushed = Some(local_hash);
                        entry.kept = None;
                        pushed.push(syncable);
                    }
                    Err(_) => failed.push(syncable.name),
                }
            }
            crate::config_sync::Plan::UpToDate => {
                let entry = state.file_mut(&ssh.host, syncable.name);
                entry.pushed = Some(local_hash);
                entry.kept = None;
            }
            crate::config_sync::Plan::Kept => kept.push(syncable.name),
            crate::config_sync::Plan::Conflict => conflicts.push(syncable.name),
        }
    }
    if let Err(e) = state.save(&state_path) {
        log(format!(
            "Config sync: could not record what was pushed ({e})"
        ));
    }
    if !kept.is_empty() {
        log(format!(
            "Config sync: keeping the remote's own {} (as you chose)",
            kept.join(", ")
        ));
    }
    for name in &conflicts {
        log(format!(
            "Config sync: {name} was changed on {host} since croft last pushed it, so it was left alone. \
             `croft sync-config {host} --diff {name}` compares the two; `--take-local {name}` pushes yours, \
             `--keep-remote {name}` keeps theirs",
            host = ssh.host
        ));
    }

    if !pushed.is_empty() {
        // Name the files rather than counting them: "4 files" tells a user
        // nothing about whether the one they just edited went.
        let names: Vec<&str> = pushed.iter().map(|s| s.name).collect();
        log(format!("Config sync: pushed {}", names.join(", ")));
        // A croft already running on the remote applies them within its
        // config watch's interval (`ConfigWatch`, #262); one started later
        // reads them at launch.
        log(format!(
            "Config sync: {} applies on the remote within a few seconds",
            names.join(", ")
        ));
    }
    if !failed.is_empty() {
        log(format!(
            "Config sync: could not push {} (the remote keeps its own copy)",
            failed.join(", ")
        ));
    }
}

/// How the user settled config files that differ on the remote (#262):
/// push these anyway, or keep the remote's copy of these.
#[derive(Debug, Default, Clone)]
pub struct SyncResolution {
    pub take_local: Vec<String>,
    pub keep_remote: Vec<String>,
}

/// `croft sync-config <host>` (#262): push the syncable config now, the
/// same way a connect does, or with `diff` set show how the remote's copy
/// of that file differs from the local one.
pub fn sync_config_now(host: &str, resolution: &SyncResolution, diff: Option<&str>) -> Result<()> {
    for name in resolution
        .take_local
        .iter()
        .chain(&resolution.keep_remote)
        .map(String::as_str)
        .chain(diff)
    {
        if !crate::config_sync::SYNCABLE.iter().any(|s| s.name == name) {
            let names: Vec<&str> = crate::config_sync::SYNCABLE
                .iter()
                .map(|s| s.name)
                .collect();
            anyhow::bail!("{name} is not a synced file ({})", names.join(", "));
        }
    }
    let ssh = SshControl::start(host)?;
    match diff {
        Some(name) => print_config_diff(&ssh, name),
        None => {
            push_config_files(&ssh, resolution, &mut |msg| println!("{msg}"));
            Ok(())
        }
    }
}

/// The remote's copy of config file `name`, empty when it has none, read
/// over the control socket at `socket_path`.
pub(super) fn read_remote_config(
    socket_path: &Path,
    host: &str,
    name: &str,
) -> std::io::Result<Vec<u8>> {
    let mut command = ssh_socket_read(
        socket_path,
        host,
        &format!("cat ~/.config/croft/{name} 2>/dev/null"),
    );
    // Waits out "Text file busy", as `read_remote_hashes` does (#2560).
    let out = crate::review_ops::spawn_retrying_busy(&mut command)
        .and_then(std::process::Child::wait_with_output)?;
    Ok(out.stdout)
}

/// Print a unified diff of the remote's copy of `name` against the local
/// one, remote first, so `+` lines are what a push would bring.
pub(super) fn print_config_diff(ssh: &SshControl, name: &str) -> Result<()> {
    let local = crate::paths::config_dir().join(name);
    let remote = read_remote_config(&ssh.socket_path, &ssh.host, name)?;
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("croft-sync-{pid}-{name}"));
    std::fs::write(&tmp, &remote)?;
    let (local_arg, empty_tmp) = if local.is_file() {
        (local, None)
    } else if cfg!(windows) {
        let empty = std::env::temp_dir().join(format!("croft-sync-empty-{pid}-{name}"));
        std::fs::write(&empty, b"")?;
        (empty.clone(), Some(empty))
    } else {
        (PathBuf::from("/dev/null"), None)
    };
    let status = match Command::new("diff")
        .args(["-u", "--label"])
        .arg(format!("{}:{name}", ssh.host))
        .arg("--label")
        .arg(format!("local:{name}"))
        .arg(&tmp)
        .arg(&local_arg)
        .status()
    {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Command::new("git")
            .args(["diff", "--no-index", "--"])
            .arg(&tmp)
            .arg(&local_arg)
            .status(),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&tmp);
    if let Some(empty) = empty_tmp {
        let _ = std::fs::remove_file(empty);
    }
    let status = status?;
    if status.code() == Some(0) {
        println!("{name} is the same on {} and here", ssh.host);
    }
    Ok(())
}

impl SyncExclusions {
    /// The exclusions as of the session start, if they read cleanly.
    pub(super) fn load() -> Self {
        let mut ex = SyncExclusions { known: None };
        ex.update(crate::config_layers::load_merged(None));
        ex
    }

    /// Re-read the exclusions: the latest clean read, or `None` while no
    /// read has been clean. User layers only, as at connect: a workspace
    /// must not decide what leaves the laptop.
    pub(super) fn current(&mut self) -> Option<(Vec<String>, Vec<String>)> {
        self.update(crate::config_layers::load_merged(None));
        self.known.clone()
    }

    pub(super) fn update(&mut self, merged: crate::config_layers::MergedConfig) {
        if merged.warnings.is_empty() {
            self.known = Some((
                merged.prefs.config_sync_excluded_hosts,
                merged.prefs.config_sync_excluded_files,
            ));
        }
    }
}

/// Appends timestamped lines to `~/.cache/croft/config-sync.log`.
pub(super) fn config_sync_log() -> impl FnMut(String) {
    let path = install_log_path().with_file_name("config-sync.log");
    move |line: String| {
        use std::io::Write as _;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "{secs} {line}");
        }
    }
}

/// Build the rsync that ships one local file to `dest`, routed and paced by
/// the bulk lane so it never queues ahead of the live session's keystrokes.
///
/// Nothing here is binary-specific: `--checksum` makes rsync compare content
/// rather than mtime, so a caller shipping config files (#262) gets
/// push-only-if-different for free and needs no stamp of its own.
pub(super) fn ship_file_rsync_command(
    lane: &crate::remote_bulk::BulkLane,
    interactive_socket: &Path,
    local: &Path,
    dest: &str,
) -> Command {
    let mut rsync = crate::proc::command("rsync");
    rsync.args(["-az", "--checksum"]);
    rsync.args(lane.rsync_throttle_args());
    rsync.arg("-e").arg(lane.rsync_ssh_arg(interactive_socket));
    rsync.arg(local).arg(dest);
    rsync
}

/// Upload `local` to `$HOME/<remote_rel>` on `host`. Uses `rsync` on Unix when
/// available, otherwise streams the file over SSH `stdin` into a temporary
/// file and atomically renames it into place (needed on native Windows, where
/// `rsync` is typically absent).
pub(super) fn ship_local_file_to_remote(
    lane: &crate::remote_bulk::BulkLane,
    interactive_socket: &Path,
    host: &str,
    local: &Path,
    remote_rel: &str,
) -> Result<()> {
    if cfg!(unix) && crate::platform::env::resolve_on_path("rsync").is_some() {
        let dest = format!("{host}:{remote_rel}");
        let status = ship_file_rsync_command(lane, interactive_socket, local, &dest)
            .status()
            .with_context(|| format!("rsyncing {} to {dest}", local.display()))?;
        anyhow::ensure!(status.success(), "rsync exited with {status}");
        return Ok(());
    }
    let file =
        std::fs::File::open(local).with_context(|| format!("opening {}", local.display()))?;
    let q_dest = shell_quote(remote_rel);
    let q_tmp = shell_quote(&format!("{remote_rel}.tmp.{}", std::process::id()));
    let remote_cmd = format!("cat > {q_tmp} && mv -f {q_tmp} {q_dest}");
    let mut cmd = lane.probe_ssh_command(interactive_socket);
    cmd.arg(host)
        .arg(&remote_cmd)
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .with_context(|| format!("streaming {} over ssh", local.display()))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh file upload failed ({}): {}", output.status, err.trim());
    }
    Ok(())
}
