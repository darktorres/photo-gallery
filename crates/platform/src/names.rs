//! Names and paths in the system's own units (`native`). The catalog and the work lists hold
//! canonical bytes (`Name`: WTF-8 on Windows, raw elsewhere); a system call is given units. Both directions are exact:
//! a name with an unpaired surrogate keeps it, on Windows and elsewhere.

use std::ffi::{OsStr, OsString};
use std::io;
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
    crate::native::PATH_RULES.parse_bytes(&canonical_of(path))
}

/// One argument's bytes, as the program's own argument list holds them: exactly on Unix, a raw byte included. On Windows,
/// its text, converted lossily, as cxx-qt-lib passes arguments to Qt (Qt reads its own arguments from the command line there,
/// exactly; the app reads its arguments from its command line, so the list only has to be well formed).
#[cfg(unix)]
pub fn arg_bytes(arg: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    arg.as_bytes().to_vec()
}

/// One argument's bytes: its text (lossy), as cxx-qt-lib passes them (see the Unix one).
#[cfg(windows)]
pub fn arg_bytes(arg: &OsStr) -> Vec<u8> {
    arg.to_string_lossy().as_bytes().to_vec()
}

/// The part of `arg` after its first `=`, as the system holds it (none without a `=`).
#[cfg(unix)]
pub fn after_first_equals(arg: &OsStr) -> Option<OsString> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let bytes = arg.as_bytes();
    let at = bytes.iter().position(|&b| b == b'=')?;
    Some(OsString::from_vec(bytes.get(at + 1..)?.to_vec()))
}

/// The part of `arg` after its first `=`, as the system holds it (none without a `=`).
#[cfg(windows)]
pub fn after_first_equals(arg: &OsStr) -> Option<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> = arg.encode_wide().collect();
    let at = units.iter().position(|&u| u == u16::from(b'='))?;
    Some(OsString::from_wide(units.get(at + 1..)?))
}

/// The real path of an existing folder, as the catalog keys it: its names in the case the file system stores them, and
/// on Windows without the verbatim prefix.
#[cfg(windows)]
pub fn real_path(root: &Path) -> io::Result<PathBuf> {
    let real = std::fs::canonicalize(root)?;
    let bytes = canonical_of(&real);
    let plain = if let Some(unc) = bytes.strip_prefix(br"\\?\UNC\") {
        [br"\\", unc].concat()
    } else if let Some(rest) = bytes.strip_prefix(br"\\?\") {
        rest.to_vec()
    } else {
        bytes
    };
    Ok(path_buf(&plain))
}

/// The real path of an existing folder, as the catalog keys it: its names in the case the file system stores them.
#[cfg(unix)]
pub fn real_path(root: &Path) -> io::Result<PathBuf> {
    stored_case(&std::fs::canonicalize(root)?)
}

/// `real` (a canonical path, with no links) with each name spelled as its folder stores it. `canonicalize` keeps the case a
/// path was typed in, so a folder typed `/mnt/usb/photos` on a case-insensitive mount (vfat, exfat, a casefold ext4
/// folder) would not match the catalog's `Photos`. Each name is looked up in its folder's listing: among the names equal to
/// it ignoring case, the one with the same device and inode is stored; no match keeps the typed name. Only the ancestors of
/// `real` are listed, never its contents.
#[cfg(unix)]
fn stored_case(real: &Path) -> io::Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    let mut typed = PathBuf::new();
    let mut stored = PathBuf::new();
    for component in real.components() {
        match component {
            Component::Normal(name) => {
                typed.push(name);
                let target = std::fs::symlink_metadata(&typed)?;
                stored.push(stored_name(&stored, name, (target.dev(), target.ino())));
            }
            other => {
                typed.push(other);
                stored.push(other);
            }
        }
    }
    Ok(stored)
}

/// The name `typed` is stored under in `folder`: the entry equal to it ignoring case, with the device and inode `identity`
/// (both from the typed name's own lookup), else `typed` itself.
#[cfg(unix)]
fn stored_name(folder: &Path, typed: &std::ffi::OsStr, identity: (u64, u64)) -> std::ffi::OsString {
    use std::os::unix::fs::MetadataExt;
    let wanted = typed.to_string_lossy().to_lowercase();
    let Ok(listing) = std::fs::read_dir(folder) else {
        return typed.to_os_string();
    };
    for entry in listing.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().to_lowercase() != wanted {
            continue;
        }
        if entry.metadata().is_ok_and(|meta| (meta.dev(), meta.ino()) == identity) {
            return name;
        }
    }
    typed.to_os_string()
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

#[cfg(all(test, unix))]
mod stored_case_tests {
    use super::*;

    /// A fresh folder for one test under the test base (`PG_TEST_DIR`, else the temp folder).
    #[expect(clippy::unwrap_used, reason = "a test helper: a folder that cannot be made fails the test")]
    fn fresh(test: &str) -> PathBuf {
        let base = std::env::var_os("PG_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("pg-stored-case-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn a_folder_typed_as_it_is_stored_is_spelled_so() {
        let base = fresh("plain");
        std::fs::create_dir_all(base.join("Photos")).unwrap();
        let real = real_path(&base.join("Photos")).unwrap();
        assert_eq!(real.file_name(), Some(std::ffi::OsStr::new("Photos")));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn two_names_that_differ_only_in_case_are_told_apart_by_identity() {
        // Where the file system keeps both (a case-sensitive one), each typed name is stored under its own spelling.
        let base = fresh("both");
        std::fs::create_dir_all(base.join("Pics")).unwrap();
        // On a case-insensitive file system the second name is the first folder, and the create fails.
        let _ = std::fs::create_dir(base.join("pics"));
        if std::fs::read_dir(&base).unwrap().count() != 2 {
            pg_testkit::skip("this test folder's file system is case-insensitive: the two names are one folder");
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }
        assert_eq!(real_path(&base.join("pics")).unwrap().file_name(), Some(std::ffi::OsStr::new("pics")));
        assert_eq!(real_path(&base.join("Pics")).unwrap().file_name(), Some(std::ffi::OsStr::new("Pics")));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_case_insensitive_folder_is_spelled_as_it_is_stored() {
        // Where the test folder is case-insensitive (vfat, exfat, a casefold ext4 folder), the typed lower-case name is
        // stored as created. Elsewhere there is no such folder to look at.
        let base = fresh("insensitive");
        std::fs::create_dir_all(base.join("Photos")).unwrap();
        if !base.join("photos").is_dir() {
            pg_testkit::skip("this test folder's file system is case-sensitive: no folder is spelled differently");
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }
        assert_eq!(real_path(&base.join("photos")).unwrap().file_name(), Some(std::ffi::OsStr::new("Photos")));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
