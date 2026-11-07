//! Paths and names as the system has them. A unit is a UTF-16 unit on Windows and a byte elsewhere, and nothing here
//! converts lossily: a name the system holds (an unpaired surrogate on NTFS, any bytes on Unix) reaches the system,
//! and the library, unchanged.
//!
//! A [`NativePath`] is what a direct Win32 call is given (the scanner's `CreateFileW`). On Windows it is NUL-terminated,
//! and in the verbatim form (`\\?\`) past the 260-character limit: std makes that form for its own calls, so it is made
//! here only for the direct ones. FFmpeg reads its files through std (ffi.rs), and ExifTool takes the plain path, as a
//! `\\?\` path isn't one it reads (its own long-name support is a separate option, FINDINGS "I/O").

/// One unit of a native name or path: a UTF-16 unit on Windows, a byte elsewhere.
#[cfg(windows)]
pub type Unit = u16;
/// One unit of a native name or path: a UTF-16 unit on Windows, a byte elsewhere.
#[cfg(unix)]
pub type Unit = u8;

/// The separator, as a unit.
#[cfg(windows)]
pub const SEP: Unit = 0x5C; // '\'
/// The separator, as a unit.
#[cfg(unix)]
pub const SEP: Unit = b'/';

/// Shorter than this, a path (a folder, or a file in one) is used as it is.
#[cfg(windows)]
const SHORT: usize = 240;

/// A path as a system call takes it: its units joined, with a NUL on Windows (and the verbatim form when it is long).
/// Its allocation is kept from one path to the next, so a pass builds its paths in one buffer.
#[derive(Default, Debug)]
pub struct NativePath(Vec<Unit>);

impl NativePath {
    /// `parts` concatenated into this path (the first starts it), replacing what it held.
    pub fn set(&mut self, parts: &[&[Unit]]) {
        self.0.clear();
        set_units(&mut self.0, parts);
    }

    /// The units the call is given: NUL-terminated on Windows.
    pub fn units(&self) -> &[Unit] {
        &self.0
    }
}

/// `path` set to an OS string as the system holds it: its units, exact (where a `&str` is the UTF-8 form of them).
pub fn set_os_str(path: &mut NativePath, text: &std::ffi::OsStr) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u16> = text.encode_wide().collect();
        path.set(&[&units]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.set(&[text.as_bytes()]);
    }
}

#[cfg(windows)]
fn set_units(out: &mut Vec<Unit>, parts: &[&[Unit]]) {
    let (first, tail) = parts.split_first().map_or((&[][..], &[][..]), |(f, t)| (*f, t));
    let len: usize = parts.iter().map(|p| p.len()).sum();
    let (prefix, rest) = split(first, len);
    out.extend_from_slice(prefix);
    out.extend_from_slice(rest);
    for part in tail {
        out.extend_from_slice(part);
    }
    out.push(0);
}

#[cfg(unix)]
fn set_units(out: &mut Vec<Unit>, parts: &[&[Unit]]) {
    for part in parts {
        out.extend_from_slice(part);
    }
}

/// The units of the verbatim prefixes, and of a device path (`\\.\`), which is left as it is.
#[cfg(windows)]
const VERBATIM: &[Unit] = &[0x5C, 0x5C, 0x3F, 0x5C]; // \\?\
#[cfg(windows)]
const VERBATIM_UNC: &[Unit] = &[
    0x5C, // \
    0x5C, // \
    0x3F, // ?
    0x5C, // \
    0x55, // U
    0x4E, // N
    0x43, // C
    0x5C, // \
];
#[cfg(windows)]
const DEVICE: &[Unit] = &[0x5C, 0x5C, 0x2E, 0x5C]; // \\.\

/// What the verbatim form of a path `len` units long that starts with `first` puts first, and the part of `first`
/// that follows it (the leading `\\` of a share gives way to `\\?\UNC\`).
#[cfg(windows)]
fn split(first: &[Unit], len: usize) -> (&'static [Unit], &[Unit]) {
    if len < SHORT || first.starts_with(VERBATIM) || first.starts_with(DEVICE) {
        (&[], first)
    } else if let Some(unc) = first.strip_prefix(&[0x5C, 0x5C][..]) {
        (VERBATIM_UNC, unc)
    } else {
        (VERBATIM, first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn wide(s: &str) -> Vec<Unit> {
        s.encode_utf16().collect()
    }

    /// `s` as the system is given it: UTF-16 with its NUL.
    #[cfg(windows)]
    fn nul(s: &str) -> Vec<Unit> {
        wide(s).into_iter().chain([0]).collect()
    }

    #[cfg(windows)]
    #[test]
    fn short_paths_are_used_as_they_are() {
        let mut path = NativePath::default();
        path.set(&[&wide(r"D:\a")[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(r"D:\a\b.jpg"));
    }

    #[cfg(windows)]
    #[test]
    fn long_paths_are_verbatim_and_shares_are_unc() {
        let mut path = NativePath::default();
        let dir = "x".repeat(250);
        path.set(&[&wide(&format!(r"D:\{dir}"))[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(&format!(r"\\?\D:\{dir}\b.jpg")));
        path.set(&[&wide(&format!(r"\\nas\photos\{dir}"))[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(&format!(r"\\?\UNC\nas\photos\{dir}\b.jpg")));
        // Already verbatim, or a device path: as it is.
        let already = format!(r"\\?\D:\{dir}");
        path.set(&[&wide(&already)[..]]);
        assert_eq!(path.units(), nul(&already));
        let device = format!(r"\\.\D:\{dir}");
        path.set(&[&wide(&device)[..]]);
        assert_eq!(path.units(), nul(&device));
    }

    #[cfg(windows)]
    #[test]
    fn the_tail_counts_toward_the_limit() {
        // A folder just short of the limit is long once its separator and name are counted.
        let edge = format!(r"D:\{}", "x".repeat(SHORT - 4));
        assert_eq!(edge.len(), SHORT - 1);
        let mut path = NativePath::default();
        path.set(&[&wide(&edge)[..], &[SEP][..], &wide("c.jpg")[..]]);
        let units = path.units();
        assert!(String::from_utf16(&units[..units.len() - 1]).unwrap().starts_with(r"\\?\D:"));
    }

    #[cfg(unix)]
    #[test]
    fn units_are_the_bytes_as_they_are() {
        let mut path = NativePath::default();
        path.set(&[&b"/a/"[..], &[0xFF, b'b'][..]]);
        assert_eq!(path.units(), b"/a/\xFFb");
    }
}
