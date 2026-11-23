//! Cross-platform host terminal capability probing (DA1, Kitty keyboard,
//! XTWINOPS pixel geometry) and Windows console fallback helpers.
//!
//! All `unsafe` Win32 console FFI is confined to the `#[cfg(windows)]` section
//! of this module.

/// Combined result of probing the host terminal at startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostProbeResult {
    /// Host DA1 response advertised attribute `4` (Sixel graphics).
    pub sixel: bool,
    /// Host answered the Kitty keyboard protocol query (`CSI ? u`).
    pub kitty_keyboard: bool,
    /// Host sent any recognizable capability or geometry reply.
    pub answered: bool,
    /// Character cell dimensions in pixels `(width, height)`.
    pub cell_pixel_size: Option<(u16, u16)>,
    /// Window / text area dimensions in pixels `(width, height)`.
    pub window_pixel_size: Option<(u16, u16)>,
}

/// Return `true` when `bytes` contains a complete Primary Device Attributes
/// response (`CSI ? <digits/semicolons> c`).
pub fn has_complete_da1(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' && bytes[i + 2] == b'?' {
            let mut j = i + 3;
            let param_start = j;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            if j > param_start && j < bytes.len() && bytes[j] == b'c' {
                return true;
            }
            i = if j < bytes.len() && bytes[j] == 0x1b {
                j
            } else {
                i + 2
            };
        } else {
            i += 1;
        }
    }
    false
}

/// Parse host terminal probe reply bytes without a fallback character grid.
#[allow(dead_code)]
pub fn parse_host_probe_bytes(bytes: &[u8]) -> HostProbeResult {
    parse_host_probe_bytes_with_grid(bytes, None)
}

/// Parse host terminal probe reply bytes (`\x1b[?u`, `CSI 16 t`, `CSI 14 t`,
/// DA1 `\x1b[?...c`), deriving `cell_pixel_size` from `window_pixel_size` and
/// `grid` `(cols, rows)` when `CSI 16 t` was not answered.
pub fn parse_host_probe_bytes_with_grid(bytes: &[u8], grid: Option<(u16, u16)>) -> HostProbeResult {
    let mut out = HostProbeResult::default();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] != 0x1b || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }

        let mut j = i + 2;
        let private = if j < bytes.len() && matches!(bytes[j], b'?' | b'>' | b'=' | b'<') {
            let p = bytes[j];
            j += 1;
            Some(p)
        } else {
            None
        };

        let param_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';' || bytes[j] == b':')
        {
            j += 1;
        }
        let params = &bytes[param_start..j];

        let inter_start = j;
        while j < bytes.len() && (0x20..=0x2F).contains(&bytes[j]) {
            j += 1;
        }
        let has_intermediates = j > inter_start;

        if j < bytes.len() && (0x40..=0x7E).contains(&bytes[j]) {
            let final_byte = bytes[j];
            if !has_intermediates {
                apply_probe_csi(private, params, final_byte, &mut out);
            }
            i = j + 1;
        } else if j < bytes.len() && bytes[j] == 0x1b {
            i = j;
        } else {
            i += 2;
        }
    }

    if out.cell_pixel_size.is_none()
        && let (Some((win_w, win_h)), Some((cols, rows))) = (out.window_pixel_size, grid)
        && cols > 0
        && rows > 0
    {
        let cell_w = win_w / cols;
        let cell_h = win_h / rows;
        if cell_w > 0 && cell_h > 0 {
            out.cell_pixel_size = Some((cell_w, cell_h));
        }
    }

    out
}

fn apply_probe_csi(private: Option<u8>, params: &[u8], final_byte: u8, out: &mut HostProbeResult) {
    match (private, final_byte) {
        // Kitty keyboard protocol query reply: `CSI ? <flags> u`.
        (Some(b'?'), b'u') => {
            if !params.is_empty() && params.iter().all(|&b| b.is_ascii_digit()) {
                out.kitty_keyboard = true;
                out.answered = true;
            }
        }
        // Primary Device Attributes reply: `CSI ? <attr>;... c`.
        (Some(b'?'), b'c') => {
            if !params.is_empty() && params.iter().all(|&b| b.is_ascii_digit() || b == b';') {
                out.answered = true;
                if params.split(|&b| b == b';').any(|p| p == b"4") {
                    out.sixel = true;
                }
            }
        }
        // XTWINOPS geometry replies:
        //   CSI 16 t -> CSI 6 ; <height> ; <width> t  (cell pixel size)
        //   CSI 14 t -> CSI 4 ; <height> ; <width> t  (window pixel size)
        (None, b't') => {
            if let Some((kind, height, width)) = parse_three_u16_params(params)
                && width > 0
                && height > 0
            {
                match kind {
                    6 => {
                        out.cell_pixel_size = Some((width, height));
                        out.answered = true;
                    }
                    4 => {
                        out.window_pixel_size = Some((width, height));
                        out.answered = true;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn parse_three_u16_params(params: &[u8]) -> Option<(u16, u16, u16)> {
    let mut parts = params.split(|&b| b == b';');
    let a = parse_u16(parts.next()?)?;
    let b = parse_u16(parts.next()?)?;
    let c = parse_u16(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

fn parse_u16(slice: &[u8]) -> Option<u16> {
    if slice.is_empty() {
        return None;
    }
    std::str::from_utf8(slice).ok()?.parse::<u16>().ok()
}

/// Query the active Windows console font pixel dimensions via
/// `GetCurrentConsoleFontEx`. Returns `None` outside Windows or when standard
/// output is not attached to a real console reporting non-zero font metrics.
pub fn console_font_cell_pixel() -> Option<(u16, u16)> {
    #[cfg(windows)]
    {
        win::console_font_cell_pixel()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Pending host terminal probe handle returned by [`ask_host_terminal`].
#[cfg(windows)]
pub struct PendingWindowsProbe(win::PendingProbeState);

/// Stub pending probe on non-Windows hosts; `src/iterm2_inline.rs` uses the
/// Unix termios/poll path on Unix.
#[cfg(not(windows))]
pub struct PendingWindowsProbe;

impl PendingWindowsProbe {
    /// Wait up to the probe deadline for VT replies (breaking early on DA1),
    /// restore any modified Windows console modes, re-queue any unrelated user
    /// input events, and parse the collected capability response.
    pub fn answer(self) -> HostProbeResult {
        #[cfg(windows)]
        {
            self.0.answer()
        }
        #[cfg(not(windows))]
        {
            HostProbeResult::default()
        }
    }
}

/// Send Kitty keyboard (`\x1b[?u`), XTWINOPS (`\x1b[16t`, `\x1b[14t`), and DA1
/// (`\x1b[c`) probe sequences to the active Windows console without blocking.
/// On non-Windows or non-console handles this returns an inert probe whose
/// [`PendingWindowsProbe::answer`] resolves immediately to
/// [`HostProbeResult::default`].
pub fn ask_host_terminal() -> PendingWindowsProbe {
    #[cfg(windows)]
    {
        PendingWindowsProbe(win::ask_host_terminal())
    }
    #[cfg(not(windows))]
    {
        PendingWindowsProbe
    }
}

/// Query the visible console grid `(columns, rows)` from the Windows console
/// screen buffer viewport (`GetConsoleScreenBufferInfo`).
#[cfg(windows)]
#[allow(dead_code)]
pub fn console_grid_size() -> Option<(u16, u16)> {
    win::console_grid_size()
}

#[cfg(windows)]
mod win {
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Console::{
        CONSOLE_FONT_INFOEX, CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT,
        ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetConsoleScreenBufferInfo,
        GetCurrentConsoleFontEx, GetStdHandle, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode, WriteConsoleA, WriteConsoleInputW,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    use super::{HostProbeResult, has_complete_da1, parse_host_probe_bytes_with_grid};

    const PROBE_QUERY: &[u8] = b"\x1b[?u\x1b[16t\x1b[14t\x1b[c";
    const PROBE_BUDGET: Duration = Duration::from_millis(150);
    const MAX_CSI_SEQ_LEN: usize = 128;

    fn valid_handle(h: HANDLE) -> Option<HANDLE> {
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(h)
        }
    }

    pub(super) fn console_grid_size() -> Option<(u16, u16)> {
        let stdout_h = valid_handle(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) })?;
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        if unsafe { GetConsoleScreenBufferInfo(stdout_h, &mut info) } == 0 {
            return None;
        }
        let cols = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
        let rows = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
        if cols > 0 && rows > 0 {
            Some((
                cols.min(u16::MAX as i32) as u16,
                rows.min(u16::MAX as i32) as u16,
            ))
        } else {
            None
        }
    }

    pub(super) fn console_font_cell_pixel() -> Option<(u16, u16)> {
        let stdout_h = valid_handle(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) })?;
        let mut font: CONSOLE_FONT_INFOEX = unsafe { std::mem::zeroed() };
        font.cbSize = std::mem::size_of::<CONSOLE_FONT_INFOEX>() as u32;
        if unsafe { GetCurrentConsoleFontEx(stdout_h, 0, &mut font) } == 0 {
            return None;
        }
        let w = font.dwFontSize.X;
        let h = font.dwFontSize.Y;
        if w > 0 && h > 0 {
            Some((w as u16, h as u16))
        } else {
            None
        }
    }

    struct ActiveProbe {
        stdin_h: HANDLE,
        stdout_h: HANDLE,
        saved_in_mode: CONSOLE_MODE,
        saved_out_mode: CONSOLE_MODE,
        sent_at: Instant,
        modes_restored: bool,
    }

    impl ActiveProbe {
        fn restore_modes(&mut self) {
            if !self.modes_restored {
                self.modes_restored = true;
                unsafe {
                    SetConsoleMode(self.stdin_h, self.saved_in_mode);
                    SetConsoleMode(self.stdout_h, self.saved_out_mode);
                }
            }
        }
    }

    pub(super) struct PendingProbeState {
        active: Option<ActiveProbe>,
    }

    impl PendingProbeState {
        fn inert() -> Self {
            Self { active: None }
        }

        pub(super) fn answer(mut self) -> HostProbeResult {
            let Some(mut probe) = self.active.take() else {
                return HostProbeResult::default();
            };
            read_probe_answer(&mut probe)
        }
    }

    impl Drop for PendingProbeState {
        fn drop(&mut self) {
            if let Some(mut probe) = self.active.take() {
                // Drain/restore any VT probe response bytes if aborted before
                // `answer()` ran, and always restore original console modes.
                let _ = read_probe_answer(&mut probe);
            }
        }
    }

    pub(super) fn ask_host_terminal() -> PendingProbeState {
        let Some(stdin_h) = valid_handle(unsafe { GetStdHandle(STD_INPUT_HANDLE) }) else {
            return PendingProbeState::inert();
        };
        let Some(stdout_h) = valid_handle(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }) else {
            return PendingProbeState::inert();
        };

        let mut saved_in_mode: CONSOLE_MODE = 0;
        let mut saved_out_mode: CONSOLE_MODE = 0;
        if unsafe { GetConsoleMode(stdin_h, &mut saved_in_mode) } == 0
            || unsafe { GetConsoleMode(stdout_h, &mut saved_out_mode) } == 0
        {
            // Non-console handles (piped stdin/stdout, CI, redirected files):
            // return immediately without writing escape bytes or blocking.
            return PendingProbeState::inert();
        }

        let probe_in_mode = (saved_in_mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
            & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT);
        let probe_out_mode = saved_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;

        if unsafe { SetConsoleMode(stdin_h, probe_in_mode) } == 0 {
            return PendingProbeState::inert();
        }
        if unsafe { SetConsoleMode(stdout_h, probe_out_mode) } == 0 {
            unsafe {
                SetConsoleMode(stdin_h, saved_in_mode);
            }
            return PendingProbeState::inert();
        }

        let mut written: u32 = 0;
        let ok = unsafe {
            WriteConsoleA(
                stdout_h,
                PROBE_QUERY.as_ptr(),
                PROBE_QUERY.len() as u32,
                &mut written,
                std::ptr::null(),
            )
        };
        if ok == 0 || written as usize != PROBE_QUERY.len() {
            unsafe {
                SetConsoleMode(stdin_h, saved_in_mode);
                SetConsoleMode(stdout_h, saved_out_mode);
            }
            return PendingProbeState::inert();
        }

        PendingProbeState {
            active: Some(ActiveProbe {
                stdin_h,
                stdout_h,
                saved_in_mode,
                saved_out_mode,
                sent_at: Instant::now(),
                modes_restored: false,
            }),
        }
    }

    fn read_probe_answer(probe: &mut ActiveProbe) -> HostProbeResult {
        let deadline = probe.sent_at + PROBE_BUDGET;
        let mut reply_bytes = Vec::with_capacity(128);
        let mut unconsumed: Vec<INPUT_RECORD> = Vec::new();
        let mut parser = InputVtFilter::default();

        // Always do at least one 0ms check so replies that already arrived while
        // `App::new()` ran are still collected even if startup exceeded budget.
        let mut first_pass = true;
        loop {
            let timeout_ms = if first_pass {
                first_pass = false;
                0
            } else {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(u32::MAX as u128) as u32
            };

            let wait = unsafe { WaitForSingleObject(probe.stdin_h, timeout_ms) };
            if wait != WAIT_OBJECT_0 {
                if timeout_ms > 0 {
                    break;
                }
                // Initial 0ms poll found nothing yet; wait up to remaining budget.
                continue;
            }

            let mut records: [INPUT_RECORD; 64] = unsafe { std::mem::zeroed() };
            let mut count: u32 = 0;
            if unsafe {
                ReadConsoleInputW(
                    probe.stdin_h,
                    records.as_mut_ptr(),
                    records.len() as u32,
                    &mut count,
                )
            } == 0
                || count == 0
            {
                break;
            }

            for &rec in &records[..count as usize] {
                parser.push_record(rec, &mut reply_bytes, &mut unconsumed);
            }

            if has_complete_da1(&reply_bytes) {
                break;
            }
        }

        parser.flush_pending(&mut unconsumed);
        probe.restore_modes();

        if !unconsumed.is_empty() {
            let mut written: u32 = 0;
            unsafe {
                WriteConsoleInputW(
                    probe.stdin_h,
                    unconsumed.as_ptr(),
                    unconsumed.len() as u32,
                    &mut written,
                );
            }
        }

        let mut result = parse_host_probe_bytes_with_grid(&reply_bytes, console_grid_size());
        if result.cell_pixel_size.is_none() {
            result.cell_pixel_size = console_font_cell_pixel();
        }
        result
    }

    #[derive(Default)]
    struct InputVtFilter {
        esc_bytes: Vec<u8>,
        esc_records: Vec<INPUT_RECORD>,
    }

    impl InputVtFilter {
        fn flush_pending(&mut self, unconsumed: &mut Vec<INPUT_RECORD>) {
            unconsumed.append(&mut self.esc_records);
            self.esc_bytes.clear();
        }

        fn push_record(
            &mut self,
            rec: INPUT_RECORD,
            reply_bytes: &mut Vec<u8>,
            unconsumed: &mut Vec<INPUT_RECORD>,
        ) {
            if rec.EventType != KEY_EVENT as u16 {
                unconsumed.push(rec);
                return;
            }
            let key = unsafe { rec.Event.KeyEvent };
            if key.bKeyDown == 0 {
                unconsumed.push(rec);
                return;
            }
            let ch = unsafe { key.uChar.UnicodeChar };
            let repeat = key.wRepeatCount.max(1);
            if ch == 0 || ch > 0x7F {
                self.flush_pending(unconsumed);
                unconsumed.push(rec);
                return;
            }

            let byte = ch as u8;
            if self.esc_bytes.is_empty() {
                if byte == 0x1b && repeat == 1 {
                    self.esc_bytes.push(byte);
                    self.esc_records.push(rec);
                } else {
                    unconsumed.push(rec);
                }
                return;
            }

            // Currently buffering a potential ESC / CSI reply sequence.
            if byte == 0x1b && repeat == 1 {
                self.flush_pending(unconsumed);
                self.esc_bytes.push(byte);
                self.esc_records.push(rec);
                return;
            }

            if self.esc_bytes.len() == 1 {
                if byte == b'[' && repeat == 1 {
                    self.esc_bytes.push(byte);
                    self.esc_records.push(rec);
                } else {
                    self.flush_pending(unconsumed);
                    unconsumed.push(rec);
                }
                return;
            }

            if repeat != 1 || self.esc_bytes.len() >= MAX_CSI_SEQ_LEN {
                self.flush_pending(unconsumed);
                unconsumed.push(rec);
                return;
            }

            // Inside `CSI ...` sequence:
            if (0x20..=0x3F).contains(&byte) {
                self.esc_bytes.push(byte);
                self.esc_records.push(rec);
            } else if (0x40..=0x7E).contains(&byte) {
                self.esc_bytes.push(byte);
                if parse_host_probe_bytes_with_grid(&self.esc_bytes, None).answered {
                    reply_bytes.extend_from_slice(&self.esc_bytes);
                    self.esc_bytes.clear();
                    self.esc_records.clear();
                } else {
                    self.flush_pending(unconsumed);
                    unconsumed.push(rec);
                }
            } else {
                self.flush_pending(unconsumed);
                unconsumed.push(rec);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn key_down_record(ch: u8) -> INPUT_RECORD {
            let mut rec: INPUT_RECORD = unsafe { std::mem::zeroed() };
            rec.EventType = KEY_EVENT as u16;
            rec.Event.KeyEvent.bKeyDown = 1;
            rec.Event.KeyEvent.wRepeatCount = 1;
            rec.Event.KeyEvent.uChar.UnicodeChar = u16::from(ch);
            rec
        }

        #[test]
        fn filter_preserves_non_esc_user_keystrokes_and_non_probe_csi() {
            let mut filter = InputVtFilter::default();
            let mut replies = Vec::new();
            let mut unconsumed = Vec::new();

            // User types 'a', then Up arrow (`\x1b[A`), then host answers Kitty
            // (`\x1b[?0u`) and DA1 (`\x1b[?65;4c`), followed by user typing 'b'.
            let stream = b"a\x1b[A\x1b[?0u\x1b[?65;4cb";
            for &b in stream {
                filter.push_record(key_down_record(b), &mut replies, &mut unconsumed);
            }
            filter.flush_pending(&mut unconsumed);

            assert_eq!(replies, b"\x1b[?0u\x1b[?65;4c");
            let preserved: Vec<u8> = unconsumed
                .iter()
                .map(|r| unsafe { r.Event.KeyEvent.uChar.UnicodeChar as u8 })
                .collect();
            assert_eq!(preserved, b"a\x1b[Ab");
        }

        #[test]
        fn console_probe_handles_null_or_pipe_safely() {
            assert!(valid_handle(std::ptr::null_mut()).is_none());
            assert!(valid_handle(INVALID_HANDLE_VALUE).is_none());
            // Verify inert probe resolves cleanly without blocking.
            let t0 = Instant::now();
            let res = PendingProbeState::inert().answer();
            assert_eq!(res, HostProbeResult::default());
            assert!(t0.elapsed() < Duration::from_millis(50));
        }
    }
}
