//! The machine's memory: how much physical RAM there is, and how much of it is available now, so a run can bound what
//! its lanes hold together (`pg_jobs::governor::soft_cap`).

#![cfg_attr(windows, expect(unsafe_code, reason = "FFI: the Windows memory status call"))]

/// The physical memory of this machine, in bytes, or `None` when the system does not say.
pub fn physical_bytes() -> Option<u64> {
    platform::status().map(|(total, _)| total)
}

/// The memory available now without paging (Windows' `ullAvailPhys`, Linux's `MemAvailable`), in bytes, or `None` when
/// the system does not say.
pub fn available_bytes() -> Option<u64> {
    platform::status().and_then(|(_, available)| available)
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    /// The total and the available physical memory.
    pub(super) fn status() -> Option<(u64, Option<u64>)> {
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: `status` is a `MEMORYSTATUSEX` whose `dwLength` is its own size, as the call requires.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        (ok != 0).then_some((status.ullTotalPhys, Some(status.ullAvailPhys)))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    /// `MemTotal` and `MemAvailable` of `/proc/meminfo`.
    pub(super) fn status() -> Option<(u64, Option<u64>)> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        Some((meminfo(&text, "MemTotal:")?, meminfo(&text, "MemAvailable:")))
    }

    /// The value of the line `key` in bytes (the file gives kibibytes).
    pub(super) fn meminfo(text: &str, key: &str) -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(key))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        kib.checked_mul(1024)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn status() -> Option<(u64, Option<u64>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn the_machine_has_some_memory_and_some_of_it_is_available() {
        let bytes = super::physical_bytes().expect("the system reports its memory");
        assert!(bytes >= 256 << 20, "{bytes} bytes");
        let available = super::available_bytes().expect("the system reports what is available");
        assert!(available > 0 && available <= bytes, "{available} of {bytes}");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn meminfo_values_are_in_kibibytes() {
        let text = "MemFree:  100 kB\nMemTotal:       16384000 kB\nMemAvailable: 9 kB\n";
        assert_eq!(super::platform::meminfo(text, "MemTotal:"), Some(16_384_000 * 1024));
        assert_eq!(super::platform::meminfo(text, "MemAvailable:"), Some(9 * 1024));
        assert_eq!(super::platform::meminfo("MemFree: 1 kB\n", "MemTotal:"), None);
    }
}
