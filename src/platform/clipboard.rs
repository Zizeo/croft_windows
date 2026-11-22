/// Encode UTF-8 text into NUL-terminated UTF-16LE code units for Win32
/// `CF_UNICODETEXT`. Returns `None` if `text` contains an embedded NUL.
pub fn encode_utf16_nul(text: &str) -> Option<Vec<u16>> {
    if text.as_bytes().contains(&0) {
        return None;
    }
    let mut out: Vec<u16> = text.encode_utf16().collect();
    out.push(0);
    Some(out)
}

/// Decode a NUL-terminated UTF-16 slice back into UTF-8.
pub fn decode_utf16_nul(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(windows)]
struct OpenClipboardGuard;

#[cfg(windows)]
impl OpenClipboardGuard {
    fn open() -> Option<Self> {
        use windows_sys::Win32::System::DataExchange::OpenClipboard;
        for attempt in 0..5 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Self);
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        None
    }
}

#[cfg(windows)]
impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
    }
}

#[cfg(windows)]
pub fn read_string() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;

    let _guard = OpenClipboardGuard::open()?;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return None;
    }
    let byte_len = unsafe { GlobalSize(handle) };
    if byte_len < 2 {
        return Some(String::new());
    }
    let ptr = unsafe { GlobalLock(handle) } as *const u16;
    if ptr.is_null() {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, byte_len / 2) };
    let text = decode_utf16_nul(slice);
    unsafe {
        GlobalUnlock(handle);
    }
    Some(text)
}

#[cfg(windows)]
pub fn write_string(text: &str) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{EmptyClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    const CF_UNICODETEXT: u32 = 13;

    let Some(units) = encode_utf16_nul(text) else {
        return false;
    };
    let Some(_guard) = OpenClipboardGuard::open() else {
        return false;
    };
    if unsafe { EmptyClipboard() } == 0 {
        return false;
    }
    let bytes = units.len() * std::mem::size_of::<u16>();
    let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
    if hmem.is_null() {
        return false;
    }
    let ptr = unsafe { GlobalLock(hmem) } as *mut u16;
    if ptr.is_null() {
        unsafe {
            GlobalFree(hmem);
        }
        return false;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
        GlobalUnlock(hmem);
        if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
            GlobalFree(hmem);
            return false;
        }
    }
    true
}
