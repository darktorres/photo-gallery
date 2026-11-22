//! The machine's memory: how much physical RAM there is, so a run can bound what its lanes hold together.

#![cfg_attr(windows, expect(unsafe_code, reason = "FFI: the Windows memory status call"))]

/// The physical memory of this machine, in bytes, or `None` when the system does not say.
pub fn physical_bytes() -> Option<u64> {
    platform::physical_bytes()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub(super) fn physical_bytes() -> Option<u64> {
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: `status` is a `MEMORYSTATUSEX` whose `dwLength` is its own size, as the call requires.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        (ok != 0).then_some(status.ullTotalPhys)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    /// `MemTotal` of `/proc/meminfo`, which is in kibibytes.
    pub(super) fn physical_bytes() -> Option<u64> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        meminfo_total(&text)
    }

    /// The `MemTotal` line's value in bytes.
    pub(super) fn meminfo_total(text: &str) -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        kib.checked_mul(1024)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn physical_bytes() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn the_machine_has_some_memory() {
        let bytes = super::physical_bytes().expect("the system reports its memory");
        assert!(bytes >= 256 << 20, "{bytes} bytes");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn meminfo_total_is_in_kibibytes() {
        let text = "MemFree:  100 kB\nMemTotal:       16384000 kB\nMemAvailable: 9 kB\n";
        assert_eq!(super::platform::meminfo_total(text), Some(16_384_000 * 1024));
        assert_eq!(super::platform::meminfo_total("MemFree: 1 kB\n"), None);
    }
}
