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
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::io::{self, Read, Write};
    use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const MAX_TOKEN_LINE: usize = 64;
    const ACCEPT_AUTH_TIMEOUT: Duration = Duration::from_millis(100);
    const LIVENESS_PROBE_TIMEOUT: Duration = Duration::from_millis(50);

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn generate_nonce() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let pid = u64::from(std::process::id());
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut h1 = RandomState::new().build_hasher();
        h1.write_u128(nanos);
        h1.write_u64(pid);
        h1.write_u64(seq);
        let mut h2 = RandomState::new().build_hasher();
        h2.write_u64(seq.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        h2.write_u128(nanos);
        format!("{:016x}{:016x}", h1.finish(), h2.finish())
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

    fn parse_descriptor(path: &Path) -> io::Result<(u16, String)> {
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
        Ok((port, nonce.to_string()))
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

    /// Loopback TCP IPC listener backed by a capability descriptor at `path`.
    #[derive(Debug)]
    pub struct LocalListener {
        inner: TcpListener,
        path: PathBuf,
        nonce: String,
        nonblocking: AtomicBool,
    }

    /// Connected loopback TCP IPC stream authenticated with the listener's capability token.
    #[derive(Debug)]
    pub struct LocalStream {
        inner: TcpStream,
    }

    impl LocalListener {
        /// Bind a new local listener and atomically publish `<port>\n<nonce>\n` at `path`.
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

            if let Ok((existing_port, _)) = parse_descriptor(&path) {
                let addr = SocketAddr::from(([127, 0, 0, 1], existing_port));
                if TcpStream::connect_timeout(&addr, LIVENESS_PROBE_TIMEOUT).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("a listener is already serving {}", path.display()),
                    ));
                }
            }

            let inner = TcpListener::bind(("127.0.0.1", 0))?;
            let port = inner.local_addr()?.port();
            let nonce = generate_nonce();
            let payload = format!("{port}\n{nonce}\n");

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
            })
        }

        pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
            self.inner.set_nonblocking(nonblocking)?;
            self.nonblocking.store(nonblocking, Ordering::Relaxed);
            Ok(())
        }

        #[allow(dead_code)]
        pub fn local_addr(&self) -> io::Result<SocketAddr> {
            self.inner.local_addr()
        }

        pub fn accept(&self) -> io::Result<(LocalStream, ())> {
            loop {
                let (mut stream, _) = self.inner.accept()?;
                let _ = stream.set_nodelay(true);
                match self.authenticate_client(&mut stream) {
                    Ok(true) => {
                        let _ = stream.set_read_timeout(None);
                        stream.set_nonblocking(self.nonblocking.load(Ordering::Relaxed))?;
                        return Ok((LocalStream { inner: stream }, ()));
                    }
                    // Empty EOF from a concurrent `LocalListener::bind` liveness probe: skip it.
                    Ok(false) => continue,
                    Err(e) => return Err(e),
                }
            }
        }

        /// Read `<nonce>\n` one byte at a time so pipelined payload bytes remain intact
        /// in the underlying kernel socket buffer for `LocalStream` readers.
        fn authenticate_client(&self, stream: &mut TcpStream) -> io::Result<bool> {
            stream.set_nonblocking(false)?;
            let deadline = Instant::now() + ACCEPT_AUTH_TIMEOUT;
            let mut buf = Vec::with_capacity(MAX_TOKEN_LINE);
            let mut byte = [0u8; 1];

            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "invalid IPC capability token",
                    ));
                }
                stream.set_read_timeout(clamp_timeout(Some(left)))?;
                match stream.read(&mut byte) {
                    Ok(0) => {
                        if buf.is_empty() {
                            return Ok(false);
                        }
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "invalid IPC capability token",
                        ));
                    }
                    Ok(_) => {
                        if byte[0] == b'\n' {
                            break;
                        }
                        if byte[0] != b'\r' {
                            if buf.len() >= MAX_TOKEN_LINE {
                                return Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "invalid IPC capability token",
                                ));
                            }
                            buf.push(byte[0]);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "invalid IPC capability token",
                        ));
                    }
                }
            }

            if constant_time_eq(&buf, self.nonce.as_bytes()) {
                Ok(true)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "invalid IPC capability token",
                ))
            }
        }
    }

    impl Drop for LocalListener {
        fn drop(&mut self) {
            if parse_descriptor(&self.path).is_ok_and(|(_, token)| token == self.nonce) {
                let _ = std::fs::remove_file(&self.path);
                let _ = std::fs::remove_file(self.path.with_extension("bind.lock"));
            }
        }
    }

    impl LocalStream {
        pub fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
            let (port, nonce) = parse_descriptor(path.as_ref())?;
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let mut inner = TcpStream::connect(addr)?;
            let _ = inner.set_nodelay(true);
            let line = format!("{nonce}\n");
            inner.write_all(line.as_bytes())?;
            inner.flush()?;
            Ok(Self { inner })
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
