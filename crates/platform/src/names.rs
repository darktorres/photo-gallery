//! Names and paths in the system's own units (`native`). The catalog and the work lists hold
//! canonical bytes (`Name`: WTF-8 on Windows, raw elsewhere); a system call is given units. Both directions are exact:
//! a name with an unpaired surrogate keeps it, on Windows and elsewhere.

use std::path::{Path, PathBuf};

use pg_model::name::Name;

use crate::native::{NativePath, SEP, Unit};

/// A name the system holds, from its units (`scan`'s listing): exact.
pub fn name_from_native(units: &[Unit]) -> Name {
    #[cfg(windows)]
    {
        Name::from_wide(units)
    }
    #[cfg(unix)]
    {
        Name::from_raw(units)
    }
}

/// The units of canonical bytes (a name, or a whole path): WTF-8 decodes to the same UTF-16 units on Windows.
pub fn units_of(canonical: &[u8]) -> Vec<Unit> {
    #[cfg(windows)]
    {
        Name::from_raw(canonical).to_wide()
    }
    #[cfg(unix)]
    {
        canonical.to_vec()
    }
}

/// `path` set to the file `folder\name`, from the canonical bytes a work list gives (`WorkList::parts`).
pub fn set_file(path: &mut NativePath, folder: &[u8], name: &[u8]) {
    #[cfg(windows)]
    {
        let (folder, name) = (units_of(folder), units_of(name));
        path.set(&[&folder, &[SEP], &name]);
    }
    #[cfg(unix)]
    {
        path.set(&[folder, &[SEP], name]);
    }
}

/// `path` set to a whole path, from its canonical bytes (`PathRules::join`, or `WorkList::path`).
pub fn set_path(path: &mut NativePath, canonical: &[u8]) {
    path.set(&[&units_of(canonical)]);
}

/// A path for std and the programs that take one (FFmpeg, ExifTool's arguments), from its canonical bytes.
pub fn path_buf(canonical: &[u8]) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(&units_of(canonical)))
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(canonical))
    }
}

/// The canonical bytes of a whole path, as the system holds it: WTF-8 on Windows (exact, an unpaired surrogate kept),
/// raw bytes elsewhere. The inverse of [`path_buf`].
pub fn canonical_of(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        Name::from_wide(&units).as_bytes().to_vec()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
}

/// `path` under the native rules, exactly. `None` when it has no path form under them (a NUL, which no name holds).
pub fn rule_path_of(path: &Path) -> Option<pg_model::rules::Path> {
    pg_model::rules::PathRules::NATIVE.parse_bytes(&canonical_of(path))
}

/// The canonical bytes as text, for the programs that take text (ExifTool's stdin protocol, FFmpeg's `open`) and for the
/// queues that store text. Valid UTF-8 is exactly what has a UTF-8 form: a Windows name with an unpaired surrogate is
/// refused, not written back lossily.
pub fn text_of(canonical: &[u8]) -> anyhow::Result<&str> {
    std::str::from_utf8(canonical)
        .map_err(|_| anyhow::anyhow!("{} has no UTF-8 form, which this step needs", String::from_utf8_lossy(canonical)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_with_a_surrogate_keeps_it_through_the_units() {
        #[cfg(windows)]
        {
            let units: Vec<Unit> = vec![0x61, 0xD800, 0x62];
            let name = name_from_native(&units);
            assert_eq!(units_of(name.as_bytes()), units);
        }
        #[cfg(unix)]
        {
            let units: Vec<Unit> = vec![0x61, 0xFF, 0x62];
            assert_eq!(units_of(name_from_native(&units).as_bytes()), units);
        }
    }

    #[test]
    fn a_file_is_its_folder_and_name_joined() {
        #[cfg(windows)]
        const SEPARATOR: u8 = b'\\';
        #[cfg(unix)]
        const SEPARATOR: u8 = b'/';
        let folder: &[u8] = b"dir";
        let name = Name::from("b.jpg");
        let mut path = NativePath::default();
        set_file(&mut path, folder, name.as_bytes());
        let joined = [folder, &[SEPARATOR][..], b"b.jpg"].concat();
        let mut want = NativePath::default();
        set_path(&mut want, &joined);
        assert_eq!(path.units(), want.units());
    }
}
