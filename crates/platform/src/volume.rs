//! The size of the volume a folder is on, for the defaults that scale with the disk (the thumbnail cache's cap).
//! `GetDiskFreeSpaceExW` on Windows, `statvfs` on Linux (the WSL checks), and `None` elsewhere, where the caller keeps
//! its fixed default.

use std::path::Path;

/// The total size in bytes of the volume `path` is on, or `None` when it can't be read. `path` must exist.
#[cfg(windows)]
pub fn total_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut total = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; `total` is a valid out-pointer, and the two others are
    // optional, so null is allowed for them.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some(total)
}

/// The total size in bytes of the volume `path` is on, or `None` when it can't be read. `path` must exist.
#[cfg(target_os = "linux")]
pub fn total_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;

    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is NUL-terminated; `statvfs` fills `stat` when it returns 0.
    if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statvfs` returned 0, so it initialized the whole struct.
    let stat = unsafe { stat.assume_init() };
    widen(stat.f_blocks)?.checked_mul(widen(stat.f_frsize)?)
}

/// A `statvfs` count as a `u64`: the field is `u64` on some Linux targets and narrower on others.
#[cfg(target_os = "linux")]
fn widen<T: TryInto<u64>>(count: T) -> Option<u64> {
    count.try_into().ok()
}

/// The total size in bytes of the volume `path` is on, or `None` when it can't be read. `path` must exist.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn total_bytes(_path: &Path) -> Option<u64> {
    None
}
