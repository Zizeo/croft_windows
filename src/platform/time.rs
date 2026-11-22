use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 0 = Sunday ..= 6 = Saturday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Signed offset in minutes east of UTC (e.g. +120 for UTC+2).
    pub utc_offset_mins: i32,
}

#[cfg(unix)]
pub fn local_datetime(unix_secs: i64) -> Option<LocalDateTime> {
    #[allow(deprecated)]
    let secs = unix_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some(LocalDateTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        weekday: tm.tm_wday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
        utc_offset_mins: (tm.tm_gmtoff / 60) as i32,
    })
}

#[cfg(windows)]
pub fn local_datetime(unix_secs: i64) -> Option<LocalDateTime> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };
    const WINDOWS_EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    const TICKS_PER_SEC: i64 = 10_000_000;

    let ticks = unix_secs
        .checked_add(WINDOWS_EPOCH_DIFF_SECS)?
        .checked_mul(TICKS_PER_SEC)?;
    if ticks < 0 {
        return None;
    }
    let utc_u64 = ticks as u64;
    let utc_ft = FILETIME {
        dwLowDateTime: utc_u64 as u32,
        dwHighDateTime: (utc_u64 >> 32) as u32,
    };
    let mut utc_st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&utc_ft, &mut utc_st) } == 0 {
        return None;
    }
    let mut local_st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc_st, &mut local_st) } == 0 {
        return None;
    }
    let mut local_ft: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { SystemTimeToFileTime(&local_st, &mut local_ft) } == 0 {
        return None;
    }
    let local_u64 = (u64::from(local_ft.dwHighDateTime) << 32) | u64::from(local_ft.dwLowDateTime);
    let diff_ticks = local_u64 as i128 - utc_u64 as i128;
    let utc_offset_mins = (diff_ticks / (60 * TICKS_PER_SEC as i128)) as i32;

    Some(LocalDateTime {
        year: i32::from(local_st.wYear),
        month: u32::from(local_st.wMonth),
        day: u32::from(local_st.wDay),
        weekday: u32::from(local_st.wDayOfWeek),
        hour: u32::from(local_st.wHour),
        minute: u32::from(local_st.wMinute),
        second: u32::from(local_st.wSecond),
        utc_offset_mins,
    })
}

#[cfg(unix)]
pub fn thread_cpu_time() -> Option<Duration> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut ts) } == 0;
    ok.then(|| Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
}

#[cfg(windows)]
pub fn thread_cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    let to_u64 = |ft: FILETIME| (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    let total_100ns = to_u64(kernel).saturating_add(to_u64(user));
    Some(Duration::from_nanos(total_100ns.saturating_mul(100)))
}
