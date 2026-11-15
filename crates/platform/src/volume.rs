//! The size and free space of the volume a folder is on: for the defaults that scale with the disk (the thumbnail
//! cache's cap), and for the room a catalog's migration needs before it starts. `GetDiskFreeSpaceExW` on Windows,
//! `statvfs` on Linux (the Linux checks), and `None` elsewhere, where the caller keeps its fixed default or goes ahead.

use std::path::Path;

/// A volume's size and the space this process may still write to it, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    /// What the caller may write (a quota, and on Linux the blocks reserved for root, are left out).
    pub free: u64,
}

/// The size and free space of the volume `path` is on, or `None` when they can't be read. `path` must exist.
#[cfg(windows)]
pub fn space(path: &Path) -> Option<Space> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: `wide` is NUL-terminated and outlives the call; `free` and `total` are valid out-pointers, and the last
    // one is optional, so null is allowed for it.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some(Space { total, free })
}

/// The size and free space of the volume `path` is on, or `None` when they can't be read. `path` must exist.
#[cfg(target_os = "linux")]
pub fn space(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;

    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is NUL-terminated; `statvfs` fills `stat` when it returns 0.
    if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statvfs` returned 0, so it initialized the whole struct.
    let stat = unsafe { stat.assume_init() };
    let block = widen(stat.f_frsize)?;
    Some(Space { total: widen(stat.f_blocks)?.checked_mul(block)?, free: widen(stat.f_bavail)?.checked_mul(block)? })
}

/// A `statvfs` count as a `u64`: the field is `u64` on some Linux targets and narrower on others.
#[cfg(target_os = "linux")]
fn widen<T: TryInto<u64>>(count: T) -> Option<u64> {
    count.try_into().ok()
}

/// The size and free space of the volume `path` is on, or `None` when they can't be read. `path` must exist.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn space(_path: &Path) -> Option<Space> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn a_real_folder_has_a_size_and_no_more_free_than_that() {
        let here = std::env::current_dir().unwrap();
        let s = space(&here).unwrap();
        assert!(s.total > 0);
        assert!(s.free <= s.total, "{s:?}");
    }
}
