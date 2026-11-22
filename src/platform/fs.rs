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
        let pathext = std::env::var("PATHEXT")
            .unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"));
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
