//! The relay's open, forward and unforward requests: opening a link in this machine's
//! browser, and forwarding a remote port to a free local one.

use super::*;

/// Validate a URL we're about to hand to the local Mac's `open(1)`. We
/// only accept the schemes the welcome panel ever produces — http, https,
/// mailto — so a hostile remote can't smuggle `file://` or shell metachars
/// through the relay log.
pub(super) fn url_is_safe_to_open(url: &str) -> bool {
    let lower: String = url
        .chars()
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase();
    let scheme_ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:");
    if !scheme_ok {
        return false;
    }
    !url.chars()
        .any(|c| c == '\0' || c == '\n' || c == '\r' || c == '\t')
}

/// Hand `url` to the local platform opener (`ShellExecuteW` on Windows, `open`
/// on macOS, `xdg-open` on Linux). The caller has already validated the URL.
pub(super) fn open_local_url(url: &str) -> std::io::Result<()> {
    crate::platform::os::open_url(url)
}

pub(super) fn handle_open_request(
    host: &str,
    socket: &Path,
    inbox_dir: &str,
    request_id: &str,
    url: &str,
) {
    if !url_is_safe_to_open(url) {
        write_relay_err(
            host,
            socket,
            inbox_dir,
            request_id,
            "rejected: URL scheme must be http/https/mailto and contain no control chars",
        );
        return;
    }
    let dest_dir = format!("{inbox_dir}/{request_id}");
    if !ssh_exec(
        host,
        socket,
        &format!("mkdir -p {}", shell_quote(&dest_dir)),
    ) {
        write_relay_err(host, socket, inbox_dir, request_id, "remote mkdir failed");
        return;
    }
    match open_local_url(url) {
        Ok(()) => {
            let _ = ssh_exec(
                host,
                socket,
                &format!("touch {}/.ok", shell_quote(&dest_dir)),
            );
        }
        Err(e) => write_relay_err(
            host,
            socket,
            inbox_dir,
            request_id,
            &format!("local open failed: {e}"),
        ),
    }
}

/// Mirror the remote port locally when it's free, else take an ephemeral port
/// from the OS. The tiny bind race (port taken between probe and `ssh -L`) just
/// surfaces as a forward failure the user can retry.
pub(super) fn pick_local_port(remote: u16) -> u16 {
    use std::net::TcpListener;
    if TcpListener::bind(("127.0.0.1", remote)).is_ok() {
        return remote;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(remote)
}

#[cfg(not(unix))]
static WINDOWS_FORWARDS: std::sync::Mutex<
    std::collections::BTreeMap<(u16, u16), crate::platform::process::ManagedChild>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Poll until the SSH forward tunnel binds its local listening socket, exits
/// early, or exceeds `timeout`.
#[cfg_attr(unix, allow(dead_code))]
fn wait_for_forward_ready(
    timeout: Duration,
    poll: Duration,
    mut check_exited: impl FnMut() -> bool,
    mut check_listening: impl FnMut() -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if check_exited() {
            return false;
        }
        if check_listening() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// Forward remote loopback `port` to the local machine by adding an `-L`
/// channel to the live SSH master (`ssh -O forward` — no second auth) on Unix
/// or spawning a tracked `ssh -N -L` tunnel on Windows, then, when `open`,
/// launch the local browser at the forwarded address.
pub(super) fn handle_forward_request(
    host: &str,
    socket: &Path,
    inbox_dir: &str,
    request_id: &str,
    port: &str,
    open: bool,
) {
    let Some(remote_port) = port.parse::<u16>().ok().filter(|p| *p >= 1) else {
        write_relay_err(
            host,
            socket,
            inbox_dir,
            request_id,
            "rejected: port must be 1..=65535",
        );
        return;
    };
    let dest_dir = format!("{inbox_dir}/{request_id}");
    if !ssh_exec(
        host,
        socket,
        &format!("mkdir -p {}", shell_quote(&dest_dir)),
    ) {
        write_relay_err(host, socket, inbox_dir, request_id, "remote mkdir failed");
        return;
    }
    let local_port = pick_local_port(remote_port);
    let spec = format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}");
    #[cfg(unix)]
    let forwarded = crate::proc::command("ssh")
        .arg("-S")
        .arg(socket)
        .arg("-O")
        .arg("forward")
        .arg("-L")
        .arg(&spec)
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    #[cfg(not(unix))]
    let forwarded = {
        let mut cmd = crate::proc::command("ssh");
        cmd.args([
            "-N",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-L",
            &spec,
            host,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        match crate::platform::process::ManagedChild::spawn_in_group(&mut cmd) {
            Ok(mut managed) => {
                let ssh_pid = managed.id();
                let ready = wait_for_forward_ready(
                    Duration::from_secs(5),
                    Duration::from_millis(50),
                    || managed.child_mut().try_wait().map_or(true, |s| s.is_some()),
                    || {
                        crate::platform::net::list_tcp_listeners().is_some_and(|list| {
                            list.iter()
                                .any(|e| e.port == local_port && e.pid == ssh_pid)
                        })
                    },
                );
                if ready {
                    if let Ok(mut map) = WINDOWS_FORWARDS.lock()
                        && let Some(mut old) = map.insert((local_port, remote_port), managed)
                    {
                        old.kill_tree();
                    }
                    true
                } else {
                    managed.kill_tree();
                    false
                }
            }
            Err(_) => false,
        }
    };
    if !forwarded {
        write_relay_err(host, socket, inbox_dir, request_id, "ssh -O forward failed");
        return;
    }
    if open {
        let _ = open_local_url(&format!("http://127.0.0.1:{local_port}/"));
    }
    let _ = ssh_exec(
        host,
        socket,
        &format!("printf %s {local_port} > {}/.ok", shell_quote(&dest_dir)),
    );
}

/// Tear down a forward added by [`handle_forward_request`]. Best-effort and
/// silent: only digits reach the `-L` spec, and a stale cancel is harmless.
pub(super) fn handle_unforward_request(host: &str, socket: &Path, local: &str, remote: &str) {
    let (Ok(local), Ok(remote)) = (local.parse::<u16>(), remote.parse::<u16>()) else {
        return;
    };
    #[cfg(unix)]
    {
        let spec = format!("127.0.0.1:{local}:127.0.0.1:{remote}");
        let _ = crate::proc::command("ssh")
            .arg("-S")
            .arg(socket)
            .arg("-O")
            .arg("cancel")
            .arg("-L")
            .arg(&spec)
            .arg(host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = (host, socket);
        if let Ok(mut map) = WINDOWS_FORWARDS.lock()
            && let Some(mut managed) = map.remove(&(local, remote))
        {
            managed.kill_tree();
        }
    }
}

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
