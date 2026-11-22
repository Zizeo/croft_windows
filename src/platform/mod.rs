//! Cross-platform OS abstractions isolating Unix (`libc` / `std::os::unix`)
//! and Windows (`windows-sys` / `dunce`) primitives behind one API.

pub mod clipboard;
pub mod env;
pub mod fs;
pub mod ipc;
pub mod os;
pub mod process;
pub mod time;
