//! Cross-platform local IPC stream/listener types.
//! On Unix these alias `std::os::unix::net`. On Windows (Sub-project 1), they
//! are inert placeholders so `App` compiles cleanly; Sub-project 5 replaces
//! them with Named Pipes / Windows `AF_UNIX`.

#[cfg(unix)]
pub type LocalListener = std::os::unix::net::UnixListener;
#[cfg(unix)]
pub type LocalStream = std::os::unix::net::UnixStream;

#[cfg(windows)]
#[derive(Debug)]
pub struct LocalListener;

#[cfg(windows)]
#[derive(Debug)]
pub struct LocalStream;
