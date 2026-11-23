//! Multiplayer session host: croft's own replacement for dtach on the
//! persistent-session path (see docs/MULTIPLAYER.md).
//!
//! One `session-host` server owns the PTY the real croft TUI runs on and
//! accepts any number of clients over a unix socket. Output bytes are
//! broadcast to every client verbatim (byte-transparent, like dtach: Kitty
//! graphics and every escape sequence pass through untouched). Input is
//! attributed: each client is its own socket connection, so the server knows
//! who typed what and can enforce write control server-side, which dtach and
//! abduco cannot. Window size is the minimum across connected clients (the
//! tmux rule), re-asserted with a jiggle on attach so the inner croft always
//! repaints for a reattaching client.
//!
//! Wire format, both directions: `[type: u8][len: u32 be][payload]`.
//! Type 0 carries raw PTY bytes; type 1 carries one NDJSON-style control
//! message (see [`Control`]), the same compact-JSON convention as
//! `crate::mcp::transport`.

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
#[cfg(unix)]
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod attach;
#[cfg(unix)]
mod connection;
#[cfg(unix)]
mod detach_chord;
#[cfg(unix)]
mod frame_write;
mod identity;
#[cfg(unix)]
mod inner_channel;
#[cfg(unix)]
mod launch;
#[cfg(unix)]
mod outbox;
mod protocol;
#[cfg(unix)]
mod pty;
mod roster;
#[cfg(unix)]
mod server;
mod sidecars;
#[cfg(unix)]
mod swap;
#[cfg(unix)]
use attach::*;
#[cfg(unix)]
use connection::*;
#[cfg(unix)]
use detach_chord::*;
#[cfg(unix)]
pub(crate) use frame_write::WRITE_FRAME_DEADLINE;
#[cfg(unix)]
pub(crate) use frame_write::write_frame_blocking;
#[cfg(unix)]
use frame_write::*;
pub(crate) use identity::client_name;
#[cfg(unix)]
use identity::*;
#[cfg(unix)]
pub use launch::attach_or_create;
#[cfg(all(unix, test))]
use launch::*;
#[cfg(unix)]
use outbox::*;
#[cfg(any(unix, test))]
pub use protocol::Control;
#[cfg(any(unix, test))]
pub use protocol::Frame;
#[cfg(unix)]
pub use protocol::FrameReader;
pub use protocol::Participant;
pub use protocol::encode_bytes_frame;
#[cfg(unix)]
pub use protocol::encode_control_frame;
#[cfg(all(unix, test))]
use protocol::*;
#[cfg(unix)]
use pty::*;
#[cfg(any(unix, test))]
pub use roster::min_winsize;
pub use roster::session_version;
pub use roster::sole_detach_target;
pub use roster::stale_client_notice;
pub use roster::stale_client_notice_on_change;
use roster::*;
#[cfg(unix)]
pub use server::serve;
#[cfg(all(unix, test))]
pub(crate) use server::serve_with_token;
#[cfg(unix)]
use server::*;
pub use sidecars::read_host_version;
pub use sidecars::read_presence;
#[cfg(unix)]
pub use sidecars::stale_marker_path;
#[cfg(unix)]
use sidecars::*;
#[cfg(unix)]
use swap::*;

/// The raw handles a serve wrapper must carry across its own exec: the
/// listening socket (so the address never unbinds and reconnects queue in
/// the backlog during the swap), the PTY master (so the inner croft never
/// sees HUP), and the inner child's pid (exec preserves the parent/child
/// relationship, so the successor can still wait on it).
#[cfg(unix)]
struct HandoffFds {
    listener: std::os::fd::RawFd,
    master: std::os::fd::RawFd,
    child_pid: u32,
}

/// A client's outbound half: a bounded queue drained by that client's own
/// writer thread.
///
/// Senders ([`broadcast`], and every control frame the server originates)
/// only ever push here and wake the writer — they never block on the peer's
/// socket, which is what kept one wedged client from freezing everyone
/// else. When the queue exceeds [`CLIENT_QUEUE_LIMIT`] the client is marked
/// dead and its queue dropped; the writer thread then exits and the client
/// thread deregisters it, exactly as for a peer that closed the connection.
#[cfg(unix)]
struct Outbox {
    queue: Mutex<OutboxState>,
    wake: std::sync::Condvar,
}

#[cfg(unix)]
#[derive(Default)]
struct OutboxState {
    /// Frames awaiting delivery, oldest first.
    pending: std::collections::VecDeque<Vec<u8>>,
    /// Total bytes held in `pending`, tracked incrementally so the overflow
    /// check stays O(1) on the pump's path.
    bytes: usize,
    /// Set once the peer is gone (closed, wedged past the queue limit, or
    /// deliberately disconnected). Latches: a dead outbox never revives.
    dead: bool,
    /// Set once nothing further will be queued: the writer delivers what is
    /// already pending and then exits, instead of parking forever on a
    /// connection nobody owns (#321).
    closing: bool,
    /// Frames this outbox has accepted, and how many of them its writer has
    /// actually put on the wire. Queueing is not delivery: a writer can be
    /// parked mid-frame on a slow peer for up to [`WRITE_FRAME_DEADLINE`],
    /// so the only way to wait for ONE specific frame to land is to
    /// remember its number here and watch `delivered` reach it (#321).
    queued: u64,
    delivered: u64,
}

#[cfg(unix)]
struct Client {
    id: u64,
    name: String,
    /// The version the client's Hello carried (empty when it sent none).
    version: String,
    cols: u16,
    rows: u16,
    control: bool,
    /// Owning handle on the socket. Nothing reads it any more — output goes
    /// through `outbox` and disconnects through `fd` — but it must stay:
    /// this `Arc` is what keeps the `UnixStream` alive, and therefore what
    /// keeps `fd` a valid descriptor rather than a recycled one.
    #[allow(dead_code)]
    tx: Arc<Mutex<UnixStream>>,
    /// This client's socket fd, captured at registration so a disconnect can
    /// call [`shutdown_now`] without waiting on the writer thread's lock.
    /// Valid for exactly as long as `tx` above is held.
    fd: std::os::fd::RawFd,
    /// Queued output for this client, drained by its own writer thread.
    outbox: Arc<Outbox>,
    /// Stable per-client-process identity from [`Control::Hello`], or empty
    /// when the client did not supply one. Used to evict this client's own
    /// stale registration on reconnect (#229).
    client_id: String,
}

#[cfg(unix)]
struct Host {
    clients: Mutex<Vec<Client>>,
    next_id: AtomicU64,
    pty_input: Mutex<Box<dyn Write + Send>>,
    master: Mutex<MasterHandle>,
    last_size: Mutex<(u16, u16)>,
    socket: PathBuf,
    /// Shared secret for the inner croft's privileged control channel,
    /// exported to the inner command as CROFT_SESSION_TOKEN.
    token: String,
    /// Authenticated privileged channels (the inner croft); they receive
    /// typing attribution, never PTY bytes or presence frames.
    privileged: Mutex<Vec<Arc<Mutex<UnixStream>>>>,
    /// Which client's input last reached the PTY, to send [`Control::Typing`]
    /// only on writer changes.
    last_writer: Mutex<Option<u64>>,
    /// Latched once this host has committed to replacing its own process
    /// image (#238). Every connection it has accepted dies at that `exec`,
    /// so from the latch on, a newly accepted client is told to reconnect
    /// instead of being seated (#321). Written under the `clients` lock so
    /// a registration in flight either completes (and receives the HostSwap
    /// broadcast) or sees the latch - never neither.
    swapping: AtomicBool,
    /// Goodbye frames queued for connections that arrived after the latch and
    /// were refused rather than seated. They are not in the roster, so the
    /// swap's delivery fence would not otherwise wait for them - and a refused
    /// client whose HostSwap is still queued when the exec closes its socket
    /// reads the EOF as the session ending, which is the very outcome the
    /// refusal exists to prevent (#321). Registered under the `clients` lock,
    /// so once the swap observes this empty while holding that lock, no
    /// further goodbye can appear before the exec.
    farewells: Mutex<Vec<(Arc<Outbox>, u64)>>,
}

/// The inner croft's privileged handle to its session host: lets the
/// participants UI grant/revoke write control and disconnect clients.
/// Fire-and-forget writes; the host answers through the presence sidecar,
/// which the app already polls.
#[cfg(unix)]
pub struct InnerChannel {
    stream: UnixStream,
    /// Decoder for host frames (typing attribution) on this channel.
    reader: FrameReader,
    /// The stream reported EOF or a fatal error: the host this channel
    /// authenticated to is gone (killed, or swapped to a successor image,
    /// #238). The app revives a dead channel by connecting again.
    dead: bool,
    /// The presence sidecar this session's host maintains.
    pub presence: PathBuf,
    /// The host's stale-image marker (#238): present once the host noticed
    /// its binary was replaced on disk while it kept running the old image.
    pub stale_marker: PathBuf,
    /// The host's version sidecar (#652): the version a reattaching client
    /// gets, which a host swap can move ahead of the inner croft's own.
    pub host_version: PathBuf,
}

#[cfg(not(unix))]
pub fn serve(_socket: &Path, _workspace: Option<&Path>, _inner: &[String]) -> Result<i32> {
    anyhow::bail!(
        "session-host is not supported on native Windows; run croft in WSL2 for persistent sessions"
    )
}

#[cfg(not(unix))]
pub fn attach_or_create(
    _socket: &Path,
    _workspace: Option<&Path>,
    _inner: &[String],
) -> Result<i32> {
    anyhow::bail!(
        "session-host is not supported on native Windows; run croft in WSL2 for persistent sessions"
    )
}

#[cfg(not(unix))]
#[derive(Debug)]
pub struct InnerChannel {
    pub presence: PathBuf,
    pub stale_marker: PathBuf,
    pub host_version: PathBuf,
}

#[cfg(not(unix))]
#[cfg_attr(not(unix), allow(dead_code))]
impl InnerChannel {
    pub fn from_env() -> Option<Self> {
        None
    }

    pub fn connect(_socket: &Path, _token: &str) -> Option<Self> {
        None
    }

    pub fn drain_typing(&mut self) -> Vec<u64> {
        Vec::new()
    }

    pub fn is_dead(&self) -> bool {
        true
    }

    pub fn set_control(&mut self, _id: u64, _grant: bool) -> bool {
        false
    }

    pub fn kick(&mut self, _id: u64) -> bool {
        false
    }
}

#[cfg(all(test, unix))]
mod tests;
