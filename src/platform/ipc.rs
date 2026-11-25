//! Cross-platform local IPC stream/listener types.
//!
//! On Unix these alias `std::os::unix::net`. On Windows they use loopback TCP
//! (`127.0.0.1:0`) paired with a capability token file at `path` so `croft`
//! gets identical synchronous, non-blocking (`WouldBlock`), timeout, and
//! `try_clone` semantics without `AF_UNIX` 108-byte path limits or `unsafe`
//! overlapped Named Pipe FFI.

#[cfg(unix)]
pub type LocalListener = std::os::unix::net::UnixListener;
#[cfg(unix)]
pub type LocalStream = std::os::unix::net::UnixStream;

#[cfg(windows)]
mod win {
    use std::fmt::Write as _;
    use std::io::{self, Read, Write};
    use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use ring::rand::{SecureRandom, SystemRandom};

    const MAX_TOKEN_LINE: usize = 64;
    const MAX_PENDING_AUTH: usize = 32;
    const ACCEPT_AUTH_TIMEOUT: Duration = Duration::from_millis(100);
    const LIVENESS_PROBE_TIMEOUT: Duration = Duration::from_millis(50);

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn generate_nonce() -> io::Result<String> {
        let mut raw = [0u8; 16];
        SystemRandom::new()
            .fill(&mut raw)
            .map_err(|_| io::Error::other("CSPRNG failure generating IPC nonce"))?;
        let mut nonce = String::with_capacity(raw.len() * 2);
        for b in raw {
            let _ = write!(nonce, "{b:02x}");
        }
        Ok(nonce)
    }

    fn invalid_token_err() -> io::Error {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid IPC capability token",
        )
    }

    fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
    }

    fn parse_descriptor(path: &Path) -> io::Result<(u16, String, Option<u32>)> {
        let text = std::fs::read_to_string(path)?;
        let mut lines = text.lines();
        let port = lines
            .next()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|&p| p > 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid IPC port"))?;
        let nonce = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing IPC token"))?;
        let pid = lines
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .filter(|&p| p > 0);
        Ok((port, nonce.to_string(), pid))
    }

    fn clamp_timeout(dur: Option<Duration>) -> Option<Duration> {
        dur.map(|d| {
            if d < Duration::from_millis(1) {
                Duration::from_millis(1)
            } else {
                d
            }
        })
    }

    enum AuthByte {
        Continue,
        Complete,
        Reject,
    }

    fn push_auth_byte(buf: &mut Vec<u8>, byte: u8, cr_count: &mut usize) -> AuthByte {
        if byte == b'\n' {
            return AuthByte::Complete;
        }
        if byte == b'\r' {
            *cr_count += 1;
            if *cr_count > MAX_TOKEN_LINE {
                return AuthByte::Reject;
            }
            return AuthByte::Continue;
        }
        if buf.len() >= MAX_TOKEN_LINE {
            return AuthByte::Reject;
        }
        buf.push(byte);
        AuthByte::Continue
    }

    enum PollEntry {
        Pending,
        ProbeEof,
        Rejected,
        Authenticated,
    }

    #[derive(Debug)]
    struct PendingAuth {
        stream: TcpStream,
        buf: Vec<u8>,
        deadline: Instant,
    }

    /// Loopback TCP IPC listener backed by a capability descriptor at `path`.
    #[derive(Debug)]
    pub struct LocalListener {
        inner: TcpListener,
        path: PathBuf,
        nonce: String,
        nonblocking: AtomicBool,
        pending: Mutex<Vec<PendingAuth>>,
    }

    /// Connected loopback TCP IPC stream authenticated with the listener's capability token.
    #[derive(Debug)]
    pub struct LocalStream {
        inner: TcpStream,
    }

    impl LocalListener {
        /// Bind a new local listener and atomically publish `<port>\n<nonce>\n<pid>\n` at `path`.
        pub fn bind<P: AsRef<Path>>(path: P) -> io::Result<Self> {
            let path = path.as_ref().to_path_buf();
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }

            let lock_path = path.with_extension("bind.lock");
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)?;
            super::super::fs::lock_exclusive(&lock_file)?;

            if let Ok((existing_port, _, existing_pid)) = parse_descriptor(&path) {
                let stale_dead_pid =
                    existing_pid.is_some_and(|pid| !super::super::process::is_pid_alive(pid));
                if !stale_dead_pid {
                    let addr = SocketAddr::from(([127, 0, 0, 1], existing_port));
                    if TcpStream::connect_timeout(&addr, LIVENESS_PROBE_TIMEOUT).is_ok() {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            format!("a listener is already serving {}", path.display()),
                        ));
                    }
                }
            }

            let inner = TcpListener::bind(("127.0.0.1", 0))?;
            let port = inner.local_addr()?.port();
            let nonce = generate_nonce()?;
            let payload = format!("{port}\n{nonce}\n{}\n", std::process::id());

            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = path.with_extension(format!("tmp.{}.{seq}", std::process::id()));
            std::fs::write(&tmp, payload.as_bytes())?;
            if let Err(e) = super::super::fs::rename_retrying(&tmp, &path) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }

            Ok(Self {
                inner,
                path,
                nonce,
                nonblocking: AtomicBool::new(false),
                pending: Mutex::new(Vec::new()),
            })
        }

        pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
            self.inner.set_nonblocking(nonblocking)?;
            self.nonblocking.store(nonblocking, Ordering::Release);
            Ok(())
        }

        #[allow(dead_code)]
        pub fn local_addr(&self) -> io::Result<SocketAddr> {
            self.inner.local_addr()
        }

        pub fn accept(&self) -> io::Result<(LocalStream, ())> {
            if self.nonblocking.load(Ordering::Acquire) {
                self.accept_nonblocking()
            } else {
                self.accept_blocking()
            }
        }

        fn finish_authenticated_stream(&self, stream: TcpStream) -> io::Result<LocalStream> {
            let _ = stream.set_read_timeout(None);
            stream.set_nonblocking(self.nonblocking.load(Ordering::Acquire))?;
            Ok(LocalStream { inner: stream })
        }

        fn accept_nonblocking(&self) -> io::Result<(LocalStream, ())> {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());

            // 1. Drain newly accepted sockets into `pending` (non-blocking + nodelay).
            //    If `pending.len() >= MAX_PENDING_AUTH`, prune expired entries first;
            //    if still full, serve a ready oldest entry if authenticated or drop the
            //    oldest pending entry to stay bounded without starving legitimate clients.
            loop {
                match self.inner.accept() {
                    Ok((stream, _)) => {
                        if stream.set_nonblocking(true).is_err() {
                            continue;
                        }
                        let _ = stream.set_nodelay(true);
                        if pending.len() >= MAX_PENDING_AUTH {
                            let now = Instant::now();
                            pending.retain(|entry| now < entry.deadline);
                            if pending.len() >= MAX_PENDING_AUTH
                                && matches!(
                                    self.poll_pending_entry(&mut pending[0]),
                                    PollEntry::Authenticated
                                )
                            {
                                let entry = pending.remove(0);
                                pending.push(PendingAuth {
                                    stream,
                                    buf: Vec::with_capacity(MAX_TOKEN_LINE),
                                    deadline: Instant::now() + ACCEPT_AUTH_TIMEOUT,
                                });
                                return Ok((self.finish_authenticated_stream(entry.stream)?, ()));
                            }
                            if pending.len() >= MAX_PENDING_AUTH {
                                pending.remove(0);
                            }
                        }
                        pending.push(PendingAuth {
                            stream,
                            buf: Vec::with_capacity(MAX_TOKEN_LINE),
                            deadline: Instant::now() + ACCEPT_AUTH_TIMEOUT,
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        if pending.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                }
            }

            // 2. Walk `pending` from index 0 so silent peers do not stall ready clients behind them.
            let mut idx = 0;
            while idx < pending.len() {
                match self.poll_pending_entry(&mut pending[idx]) {
                    PollEntry::Pending => idx += 1,
                    // Empty EOF from a concurrent `LocalListener::bind` liveness probe: drop and continue.
                    PollEntry::ProbeEof => {
                        pending.remove(idx);
                    }
                    PollEntry::Rejected => {
                        pending.remove(idx);
                        return Err(invalid_token_err());
                    }
                    PollEntry::Authenticated => {
                        let entry = pending.remove(idx);
                        return Ok((self.finish_authenticated_stream(entry.stream)?, ()));
                    }
                }
            }

            // 3. No entry completed in this poll pass.
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "no authenticated IPC client ready",
            ))
        }

        fn poll_pending_entry(&self, entry: &mut PendingAuth) -> PollEntry {
            if Instant::now() >= entry.deadline {
                return PollEntry::Rejected;
            }
            let mut byte = [0u8; 1];
            let mut cr_count = 0usize;
            loop {
                if Instant::now() >= entry.deadline {
                    return PollEntry::Rejected;
                }
                match entry.stream.read(&mut byte) {
                    Ok(0) => {
                        return if entry.buf.is_empty() {
                            PollEntry::ProbeEof
                        } else {
                            PollEntry::Rejected
                        };
                    }
                    Ok(_) => match push_auth_byte(&mut entry.buf, byte[0], &mut cr_count) {
                        AuthByte::Continue => {}
                        AuthByte::Complete => {
                            return if constant_time_eq(&entry.buf, self.nonce.as_bytes()) {
                                PollEntry::Authenticated
                            } else {
                                PollEntry::Rejected
                            };
                        }
                        AuthByte::Reject => return PollEntry::Rejected,
                    },
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => return PollEntry::Pending,
                    Err(_) => return PollEntry::Rejected,
                }
            }
        }

        fn accept_blocking(&self) -> io::Result<(LocalStream, ())> {
            loop {
                let leftover = {
                    let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                    if pending.is_empty() {
                        None
                    } else {
                        Some(pending.remove(0))
                    }
                };

                let mut entry = match leftover {
                    Some(pending_entry) => pending_entry,
                    None => {
                        let (stream, _) = self.inner.accept()?;
                        let _ = stream.set_nodelay(true);
                        PendingAuth {
                            stream,
                            buf: Vec::with_capacity(MAX_TOKEN_LINE),
                            deadline: Instant::now() + ACCEPT_AUTH_TIMEOUT,
                        }
                    }
                };

                match self.authenticate_client(&mut entry) {
                    Ok(true) => return Ok((self.finish_authenticated_stream(entry.stream)?, ())),
                    // Empty EOF from a concurrent `LocalListener::bind` liveness probe: skip it.
                    Ok(false) => continue,
                    Err(e) => return Err(e),
                }
            }
        }

        /// Read `<nonce>\n` one byte at a time so pipelined payload bytes remain intact
        /// in the underlying kernel socket buffer for `LocalStream` readers.
        fn authenticate_client(&self, entry: &mut PendingAuth) -> io::Result<bool> {
            entry.stream.set_nonblocking(false)?;
            let mut byte = [0u8; 1];
            let mut cr_count = 0usize;

            loop {
                let left = entry.deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(invalid_token_err());
                }
                entry.stream.set_read_timeout(clamp_timeout(Some(left)))?;
                match entry.stream.read(&mut byte) {
                    Ok(0) => {
                        if entry.buf.is_empty() {
                            return Ok(false);
                        }
                        return Err(invalid_token_err());
                    }
                    Ok(_) => match push_auth_byte(&mut entry.buf, byte[0], &mut cr_count) {
                        AuthByte::Continue => {}
                        AuthByte::Complete => break,
                        AuthByte::Reject => return Err(invalid_token_err()),
                    },
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(invalid_token_err()),
                }
            }

            if constant_time_eq(&entry.buf, self.nonce.as_bytes()) {
                Ok(true)
            } else {
                Err(invalid_token_err())
            }
        }
    }

    impl Drop for LocalListener {
        fn drop(&mut self) {
            if parse_descriptor(&self.path).is_ok_and(|(_, token, _)| token == self.nonce) {
                let _ = std::fs::remove_file(&self.path);
                let _ = std::fs::remove_file(self.path.with_extension("bind.lock"));
            }
        }
    }

    impl LocalStream {
        pub fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
            let (port, nonce, _) = parse_descriptor(path.as_ref())?;
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let mut inner = TcpStream::connect(addr)?;
            let _ = inner.set_nodelay(true);
            let line = format!("{nonce}\n");
            inner.write_all(line.as_bytes())?;
            inner.flush()?;
            Ok(Self { inner })
        }

        #[cfg_attr(not(test), allow(dead_code))]
        pub fn pair() -> io::Result<(Self, Self)> {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            let addr = listener.local_addr()?;
            let a = TcpStream::connect(addr)?;
            let (b, _) = listener.accept()?;
            let _ = a.set_nodelay(true);
            let _ = b.set_nodelay(true);
            Ok((Self { inner: a }, Self { inner: b }))
        }

        pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
            self.inner.set_nonblocking(nonblocking)
        }

        pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
            self.inner.set_read_timeout(clamp_timeout(dur))
        }

        pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
            self.inner.set_write_timeout(clamp_timeout(dur))
        }

        #[allow(dead_code)]
        pub fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                inner: self.inner.try_clone()?,
            })
        }

        #[allow(dead_code)]
        pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
            self.inner.shutdown(how)
        }
    }

    impl Read for LocalStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Read for &LocalStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&self.inner).read(buf)
        }
    }

    impl Write for LocalStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.inner.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    impl Write for &LocalStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.inner).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.inner).flush()
        }
    }
}

#[cfg(windows)]
pub use win::{LocalListener, LocalStream};
