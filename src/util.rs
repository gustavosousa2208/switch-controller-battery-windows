//! Small shared utilities: local time formatting and FILETIME conversion.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

pub const FILE_TIME_EPOCH_OFFSET: u64 = 11_644_473_600; // 1601-01-01 -> 1970-01-01

/// Local wall-clock time as "HH:MM:SS" (locale-independent).
pub fn local_hms() -> String {
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
}

/// Current local time as an ISO-like "YYYY-MM-DDTHH:MM:SS" (for task XML).
pub fn local_iso() -> String {
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

/// Current time as a Windows FILETIME (100 ns ticks since 1601-01-01).
pub fn filetime_now() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    (since_epoch.as_secs() + FILE_TIME_EPOCH_OFFSET) * 10_000_000
        + u64::from(since_epoch.subsec_nanos() / 100)
}

/// Convert a FILETIME value back to a SystemTime.
pub fn filetime_to_systemtime(filetime: u64) -> SystemTime {
    let secs = filetime / 10_000_000;
    if secs < FILE_TIME_EPOCH_OFFSET {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH + Duration::from_secs(secs - FILE_TIME_EPOCH_OFFSET)
}
