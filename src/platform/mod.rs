//! Cross-platform OS abstractions isolating Unix (`libc` / `std::os::unix`)
//! and Windows (`windows-sys` / `dunce`) primitives behind one API.

pub mod env;
pub mod os;
pub mod time;
