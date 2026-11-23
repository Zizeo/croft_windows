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
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"));
        pathext.split(';').any(|entry| {
            entry
                .trim()
                .strip_prefix('.')
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        })
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
#[allow(dead_code)]
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
        match std::fs::hard_link(target, link) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            Err(e) if link.symlink_metadata().is_ok() => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("link path already exists: {e}"),
            )),
            Err(_) => std::fs::copy(target, link).map(|_| ()),
        }
    }
}

/// Bounded backoff schedule (total 305 ms) for transient NTFS sharing
/// violations caused by Defender, Search Indexer, or background LSP scans.
#[cfg_attr(not(windows), allow(dead_code))]
pub const NTFS_RETRY_BACKOFF_MS: [u64; 6] = [5, 10, 20, 40, 80, 150];

/// True when `err` looks like transient Windows NTFS lock/sharing contention
/// (`ERROR_ACCESS_DENIED` = 5, `ERROR_SHARING_VIOLATION` = 32,
/// `ERROR_LOCK_VIOLATION` = 33, or `ErrorKind::PermissionDenied`) AND `target`
/// is not an intentionally read-only regular file or directory.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_transient_ntfs_contention(err: &io::Error, target: &Path) -> bool {
    if std::fs::metadata(target)
        .is_ok_and(|m| m.is_dir() || (m.is_file() && m.permissions().readonly()))
    {
        return false;
    }
    err.kind() == io::ErrorKind::PermissionDenied || matches!(err.raw_os_error(), Some(5 | 32 | 33))
}

/// Run `op`, retrying on transient NTFS sharing/access contention against
/// `target` across [`NTFS_RETRY_BACKOFF_MS`] on Windows. On non-Windows hosts,
/// runs `op()` once without sleeping.
pub fn retry_transient_io<T>(
    target: &Path,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    #[cfg(not(windows))]
    {
        let _ = target;
        op()
    }
    #[cfg(windows)]
    {
        let mut backoff = NTFS_RETRY_BACKOFF_MS.iter().copied();
        loop {
            match op() {
                Ok(val) => return Ok(val),
                Err(err) if is_transient_ntfs_contention(&err, target) => {
                    let Some(ms) = backoff.next() else {
                        return Err(err);
                    };
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                Err(err) => return Err(err),
            }
        }
    }
}

/// Rename `from` onto `to`, retrying transient NTFS sharing/access violations
/// on Windows.
pub fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    retry_transient_io(to, || std::fs::rename(from, to))
}

/// Write `bytes` to `path`, retrying transient NTFS sharing/access violations
/// on Windows.
pub fn write_retrying(path: &Path, bytes: &[u8]) -> io::Result<()> {
    retry_transient_io(path, || std::fs::write(path, bytes))
}
