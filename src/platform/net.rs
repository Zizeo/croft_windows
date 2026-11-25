//! Cross-platform TCP listener enumeration for native port detection.
//!
//! All `unsafe` Win32 IP Helper FFI is confined here behind [`list_tcp_listeners`].

/// A listening TCP socket reported by the OS kernel.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpListenerEntry {
    pub port: u16,
    pub pid: u32,
    pub is_loopback_or_wildcard: bool,
}

/// Enumerate active TCP listening sockets and their owning PIDs.
/// Returns `None` on platforms without a native implementation here.
#[cfg(windows)]
pub fn list_tcp_listeners() -> Option<Vec<TcpListenerEntry>> {
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    let mut entries = query_tcp4_listeners(AF_INET as u32)?;
    if let Some(v6) = query_tcp6_listeners(AF_INET6 as u32) {
        entries.extend(v6);
    }
    Some(entries)
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn list_tcp_listeners() -> Option<Vec<TcpListenerEntry>> {
    None
}

#[cfg(windows)]
fn fetch_tcp_table(family: u32) -> Option<(Vec<u32>, usize)> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER,
    };

    let mut size: u32 = 0;
    // SAFETY: passing null buffer with size = 0 queries the required byte length.
    let rc = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if rc != ERROR_INSUFFICIENT_BUFFER && rc != NO_ERROR {
        return None;
    }
    // Pad slightly in case a listener appears between the size query and read.
    // Back with `u32` elements so 4-byte-aligned table casts are sound.
    let words = ((size as usize).max(64) + 1024).div_ceil(4);
    let mut buf = vec![0u32; words];
    for _ in 0..2 {
        let mut buf_len = u32::try_from(buf.len().saturating_mul(4)).ok()?;
        // SAFETY: `buf` is valid writable memory of `buf_len` bytes.
        let rc = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut c_void,
                &mut buf_len,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if rc == NO_ERROR {
            let byte_len = buf_len as usize;
            if byte_len > buf.len() * std::mem::size_of::<u32>() {
                return None;
            }
            return Some((buf, byte_len));
        }
        if rc == ERROR_INSUFFICIENT_BUFFER {
            buf.resize(((buf_len as usize).saturating_add(1024)).div_ceil(4), 0);
            continue;
        }
        return None;
    }
    None
}

#[cfg(windows)]
fn is_local_or_wildcard_v6(octets: [u8; 16]) -> bool {
    let ip = std::net::Ipv6Addr::from(octets);
    if ip.is_unspecified() || ip.is_loopback() {
        return true;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4.is_unspecified() || v4.is_loopback();
    }
    false
}

#[cfg(windows)]
fn query_tcp4_listeners(family: u32) -> Option<Vec<TcpListenerEntry>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    };

    const {
        assert!(std::mem::align_of::<MIB_TCPTABLE_OWNER_PID>() <= std::mem::align_of::<u32>());
    }

    let (buf, byte_len) = fetch_tcp_table(family)?;
    if byte_len < std::mem::size_of::<u32>() {
        return None;
    }
    // SAFETY: `buf` is 4-byte aligned (`Vec<u32>`), was populated by `GetExtendedTcpTable`
    // for `MIB_TCPTABLE_OWNER_PID`, and we verify the table offset + `count * sizeof(row)`
    // fits inside `byte_len`.
    unsafe {
        let table = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        let count = (*table).dwNumEntries as usize;
        let base = buf.as_ptr() as usize;
        let rows_ptr = std::ptr::addr_of!((*table).table) as *const MIB_TCPROW_OWNER_PID;
        let offset = (rows_ptr as usize).saturating_sub(base);
        let needed = count.checked_mul(std::mem::size_of::<MIB_TCPROW_OWNER_PID>())?;
        if offset.checked_add(needed)? > byte_len {
            return None;
        }
        let rows = std::slice::from_raw_parts(rows_ptr, count);
        let mut out = Vec::with_capacity(count);
        for row in rows {
            let port = u16::from_be(row.dwLocalPort as u16);
            if port == 0 {
                continue;
            }
            let octets = row.dwLocalAddr.to_ne_bytes();
            let is_loopback_or_wildcard = octets == [0, 0, 0, 0] || octets[0] == 127;
            out.push(TcpListenerEntry {
                port,
                pid: row.dwOwningPid,
                is_loopback_or_wildcard,
            });
        }
        Some(out)
    }
}

#[cfg(windows)]
fn query_tcp6_listeners(family: u32) -> Option<Vec<TcpListenerEntry>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    };

    const {
        assert!(std::mem::align_of::<MIB_TCP6TABLE_OWNER_PID>() <= std::mem::align_of::<u32>());
    }

    let (buf, byte_len) = fetch_tcp_table(family)?;
    if byte_len < std::mem::size_of::<u32>() {
        return None;
    }
    // SAFETY: `buf` is 4-byte aligned (`Vec<u32>`), was populated by `GetExtendedTcpTable`
    // for `MIB_TCP6TABLE_OWNER_PID`, and we verify the table offset + `count * sizeof(row)`
    // fits inside `byte_len`.
    unsafe {
        let table = buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
        let count = (*table).dwNumEntries as usize;
        let base = buf.as_ptr() as usize;
        let rows_ptr = std::ptr::addr_of!((*table).table) as *const MIB_TCP6ROW_OWNER_PID;
        let offset = (rows_ptr as usize).saturating_sub(base);
        let needed = count.checked_mul(std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>())?;
        if offset.checked_add(needed)? > byte_len {
            return None;
        }
        let rows = std::slice::from_raw_parts(rows_ptr, count);
        let mut out = Vec::with_capacity(count);
        for row in rows {
            let port = u16::from_be(row.dwLocalPort as u16);
            if port == 0 {
                continue;
            }
            out.push(TcpListenerEntry {
                port,
                pid: row.dwOwningPid,
                is_loopback_or_wildcard: is_local_or_wildcard_v6(row.ucLocalAddr),
            });
        }
        Some(out)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    #[test]
    fn fetch_tcp_table_returns_properly_aligned_buffer() {
        let (v4, v4_bytes) = fetch_tcp_table(AF_INET as u32).expect("IPv4 table");
        assert!(v4_bytes <= v4.len() * std::mem::size_of::<u32>());
        assert_eq!(
            (v4.as_ptr() as usize) % std::mem::align_of::<MIB_TCPTABLE_OWNER_PID>(),
            0
        );
        if let Some((v6, v6_bytes)) = fetch_tcp_table(AF_INET6 as u32) {
            assert!(v6_bytes <= v6.len() * std::mem::size_of::<u32>());
            assert_eq!(
                (v6.as_ptr() as usize) % std::mem::align_of::<MIB_TCP6TABLE_OWNER_PID>(),
                0
            );
        }
    }
}
