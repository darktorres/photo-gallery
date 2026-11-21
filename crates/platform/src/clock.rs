//! The clock: the local time for the log, and how long the process has run for the startup trace.

/// A date and time.
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millisecond: u16,
}

/// The local time (on Windows, the system's zone).
#[cfg(windows)]
pub fn local_time() -> LocalTime {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = SYSTEMTIME::default();
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it is given.
    unsafe { GetLocalTime(&mut t) };
    LocalTime {
        year: t.wYear,
        month: t.wMonth,
        day: t.wDay,
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
        millisecond: t.wMilliseconds,
    }
}

/// Elsewhere, UTC: the zone is not read.
#[cfg(not(windows))]
pub fn local_time() -> LocalTime {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (year, month, day) = pg_model::date::civil(days);
    LocalTime {
        year: field(year),
        month: field(month),
        day: field(day),
        hour: field(rem / 3600),
        minute: field(rem % 3600 / 60),
        second: field(rem % 60),
        millisecond: field(now.subsec_millis()),
    }
}

/// The Unix time in whole seconds, as the database and the notices store it (`i64`). A clock before the epoch reads as 0,
/// and a time past `i64::MAX` seconds saturates; neither happens on a working clock.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A calendar field as the Windows API holds it (`SYSTEMTIME`'s fields are `u16`). The fallback is for a year past 65535,
/// which the clock can't reach in practice.
#[cfg(not(windows))]
fn field<T: TryInto<u16>>(value: T) -> u16 {
    value.try_into().unwrap_or(u16::MAX)
}

/// Milliseconds since the process was created (on Windows). Elsewhere, since the first call, which the startup trace
/// makes at its first stage.
#[cfg(windows)]
pub fn process_age_ms() -> f64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user, mut now) = (zero, zero, zero, zero, zero);
    // SAFETY: both only write the FILETIMEs they are given; the pseudo-handle of this process needs no closing.
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user);
        GetSystemTimePreciseAsFileTime(&mut now);
    }
    // FILETIME counts 100 ns.
    pg_model::num::size(ticks(now).saturating_sub(ticks(created))) / 1e4
}

/// The CPU cycles this process's threads have run, user and kernel (on Windows; `None` elsewhere). Unlike the wall
/// clock, it does not count the time other processes take the cores, so the profiling probes compare work with it.
#[cfg(windows)]
pub fn process_cycles() -> Option<u64> {
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    use windows_sys::Win32::System::WindowsProgramming::QueryProcessCycleTime;
    let mut cycles = 0u64;
    // SAFETY: QueryProcessCycleTime only writes the u64 it is given; the pseudo-handle of this process needs no closing.
    let ok = unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) };
    (ok != 0).then_some(cycles)
}

/// Not counted elsewhere.
#[cfg(not(windows))]
pub fn process_cycles() -> Option<u64> {
    None
}

/// Elsewhere, since the first call (see above).
#[cfg(not(windows))]
pub fn process_age_ms() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1e3
}
