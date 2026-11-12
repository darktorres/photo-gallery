//! Names and paths in the system's own units (`native`). The catalog and the work lists hold
//! canonical bytes (`Name`: WTF-8 on Windows, raw elsewhere); a system call is given units. Both directions are exact:
//! a name with an unpaired surrogate keeps it, on Windows and elsewhere.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use pg_model::name::Name;
use pg_model::rules::{Path as RulePath, PathRules};

use crate::native::{NativePath, SEP, Unit, verbatim};

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

/// A path from its canonical bytes, in the plain form. Not public: a stored path is opened by its verbatim form
/// ([`disk_path`]), and shown by `PathRules::display`, so no caller outside makes the plain form of one.
pub(crate) fn path_buf(canonical: &[u8]) -> PathBuf {
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

/// The path std, FFmpeg and ExifTool open the stored `path` by: its verbatim form (`native::verbatim`), so a folder
/// whose name ends in a dot or a space is the folder it names, not a sibling without it. The one way production code
/// turns a catalog's path into a path it opens; a path the user is shown is `PathRules::display`.
pub fn disk_path(rules: PathRules, path: &RulePath) -> PathBuf {
    disk_path_of(&rules.join(path))
}

/// [`disk_path`] of a whole path's canonical bytes, as a work list or a path arena holds them (`PathRules::join`,
/// `WorkList::path`).
pub fn disk_path_of(canonical: &[u8]) -> PathBuf {
    verbatim(&path_buf(canonical))
}

/// One name as the system holds it, from its canonical bytes (an unpaired surrogate kept on Windows, a raw byte
/// elsewhere): for a name joined to a folder the caller holds as a `Path`, as the tests' odd names are. A whole stored
/// path is [`disk_path`]'s.
pub fn os_name(name: &Name) -> OsString {
    path_buf(name.as_bytes()).into_os_string()
}

/// The canonical bytes of a whole path, as the system holds it: WTF-8 on Windows (exact, an unpaired surrogate kept),
/// raw bytes elsewhere. The inverse of the plain form a stored path's bytes make.
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

/// `path` under `rules`, exactly, the verbatim prefix dropped (`\\?\D:\a.` is `D:\a.`: the catalog keeps the plain
/// bytes, and [`disk_path`] puts the prefix back). `None` when it has no path form under them (a NUL, which no name
/// holds). The command line takes a catalog's rules; the app's are the native ones ([`rule_path_of`]).
pub fn rule_path_in(rules: PathRules, path: &Path) -> Option<RulePath> {
    rules.parse_bytes(&without_verbatim(canonical_of(path)))
}

/// [`rule_path_in`] the native rules.
pub fn rule_path_of(path: &Path) -> Option<RulePath> {
    rule_path_in(crate::native::PATH_RULES, path)
}

/// `path` as the user is shown it: without the verbatim prefix, lossy where a name has no UTF-8 form. A stored path is
/// shown by `PathRules::display`; this is for a path the app holds as a `Path` (a folder the user picked).
pub fn shown(path: &Path) -> String {
    String::from_utf8_lossy(&without_verbatim(canonical_of(path))).into_owned()
}

/// A path's canonical bytes without the verbatim prefix: `\\?\UNC\server\share` to `\\server\share`, `\\?\D:\` to
/// `D:\`. Other bytes (every path off Windows) as they are.
fn without_verbatim(bytes: Vec<u8>) -> Vec<u8> {
    if !cfg!(windows) {
        return bytes;
    }
    if let Some(unc) = bytes.strip_prefix(br"\\?\UNC\") {
        [br"\\", unc].concat()
    } else if let Some(rest) = bytes.strip_prefix(br"\\?\") {
        rest.to_vec()
    } else {
        bytes
    }
}

/// Why a path the user gave isn't one the catalog can hold ([`normalize_path`]). Its words are the client's; the
/// `Display` form is for the log.
#[derive(Debug)]
pub enum NormalizeError {
    /// The path can't be opened: gone, out of reach, or not allowed.
    Unreadable { path: PathBuf, error: io::Error },
    /// Its real path has no path form under the native rules (a NUL).
    NotAPath { path: PathBuf },
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NormalizeError::Unreadable { path, .. } => write!(f, "can't open {}", shown(path)),
            NormalizeError::NotAPath { path } => write!(f, "{} is not a path", shown(path)),
        }
    }
}

impl std::error::Error for NormalizeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NormalizeError::Unreadable { error, .. } => Some(error),
            NormalizeError::NotAPath { .. } => None,
        }
    }
}

/// The absolute path of a folder or file as the catalog has it: in its real case ([`real_path`]), without the verbatim
/// prefix and a trailing separator (a drive root is `D:`). Exact: a name with an unpaired surrogate keeps it, so a
/// folder whose real name has one (a short name such as `PROGRA~1` can resolve to it) is still the folder it is.
pub fn normalize_path(root: &Path) -> Result<RulePath, NormalizeError> {
    let real = real_path(root).map_err(|error| NormalizeError::Unreadable { path: root.to_path_buf(), error })?;
    // Trailing and doubled separators are dropped by the parse.
    rule_path_of(&real).ok_or(NormalizeError::NotAPath { path: real })
}

/// The real path of an existing folder, as the catalog keys it: its names in the case the file system stores them, and
/// on Windows without the verbatim prefix.
#[cfg(windows)]
pub fn real_path(root: &Path) -> io::Result<PathBuf> {
    let real = std::fs::canonicalize(root)?;
    Ok(path_buf(&without_verbatim(canonical_of(&real))))
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
    let Ok(listing) = std::fs::read_dir(folder) else {
        return typed.to_os_string();
    };
    let entries = listing.flatten().map(|entry| {
        let name = entry.file_name();
        (name, move || entry.metadata().ok().map(|meta| (meta.dev(), meta.ino())))
    });
    pick_stored(typed, identity, entries)
}

/// The choice of [`stored_name`] over a folder's entries, each its name and a lookup of its device and inode (made only
/// for a name equal to `typed` ignoring case): the first such entry whose identity is `identity`, else `typed`. Apart
/// from the listing, so a case-insensitive folder's lookup is tested where the test machine has none.
#[cfg(unix)]
fn pick_stored<F: FnOnce() -> Option<(u64, u64)>>(
    typed: &std::ffi::OsStr,
    identity: (u64, u64),
    entries: impl IntoIterator<Item = (std::ffi::OsString, F)>,
) -> std::ffi::OsString {
    let wanted = typed.to_string_lossy().to_lowercase();
    for (name, identity_of) in entries {
        if name.to_string_lossy().to_lowercase() == wanted && identity_of() == Some(identity) {
            return name;
        }
    }
    typed.to_os_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_whose_real_name_is_not_unicode_keeps_it() {
        // The folder is made with the name (an unpaired surrogate, or a byte no UTF-8 sequence holds), then normalized:
        // its last part is that name, exactly, where a lossy text form would have U+FFFD.
        #[cfg(windows)]
        let name: Vec<crate::native::Unit> = vec![0x61, 0xD800];
        #[cfg(unix)]
        let name: Vec<crate::native::Unit> = vec![0x61, 0xFF];
        let os = crate::testing::odd_name("a");
        let base = crate::testing::run_folder("scratch_names", "pg-normalize").unwrap();
        let folder = base.join(&os);
        std::fs::create_dir_all(&folder).unwrap();
        let path = normalize_path(&folder).unwrap();
        assert_eq!(path.parts.last(), Some(&name_from_native(&name)));
        // A path that isn't there can't be opened: the error says which, with the system's own error under it.
        let gone = base.join("gone");
        let Err(NormalizeError::Unreadable { path, error }) = normalize_path(&gone) else { panic!("not unreadable") };
        assert_eq!((path, error.kind()), (gone, io::ErrorKind::NotFound));
        std::fs::remove_dir_all(&base).unwrap();
    }
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

    /// A verbatim path is stored and shown as its plain bytes; the prefix is the opening's, not the path's.
    #[cfg(windows)]
    #[test]
    fn the_verbatim_prefix_is_not_part_of_the_path() {
        let rules = crate::native::PATH_RULES;
        for (given, plain) in [(r"\\?\D:\a.\b", r"D:\a.\b"), (r"\\?\UNC\nas\p\a.", r"\\nas\p\a."), (r"D:\a", r"D:\a")] {
            let rule = rule_path_of(Path::new(given)).unwrap();
            assert_eq!(rules.join(&rule), plain.as_bytes(), "{given}");
            assert_eq!(shown(Path::new(given)), plain, "{given}");
        }
    }

    /// A stored path reaches the file in a folder whose name ends in a dot through `disk_path`, by its rule path and
    /// by its joined bytes alike, and not the file in the sibling folder without the dot.
    #[cfg(windows)]
    #[test]
    fn a_stored_path_in_a_dotted_folder_is_reached_by_its_disk_path() {
        let dir = std::path::absolute(crate::testing::run_folder("scratch_names", "disk-path").unwrap()).unwrap();
        let dotted = dir.join("Acme Inc.");
        std::fs::create_dir_all(verbatim(&dotted)).unwrap();
        std::fs::create_dir_all(dir.join("Acme Inc")).unwrap();
        std::fs::write(verbatim(&dotted.join("a.jpg")), b"dotted").unwrap();
        std::fs::write(dir.join("Acme Inc").join("a.jpg"), b"sibling").unwrap();
        let stored = canonical_of(&dotted.join("a.jpg"));
        assert!(disk_path_of(&stored).to_string_lossy().starts_with(r"\\?\"), "the verbatim form");
        assert_eq!(std::fs::read(disk_path_of(&stored)).unwrap(), b"dotted");
        let rule = rule_path_of(&dotted.join("a.jpg")).unwrap();
        assert_eq!(std::fs::read(disk_path(crate::native::PATH_RULES, &rule)).unwrap(), b"dotted");
        std::fs::remove_dir_all(verbatim(&dir)).unwrap();
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

    /// A fresh folder for one test: a run folder of its own under the test base.
    #[expect(clippy::unwrap_used, reason = "a test helper: a folder that cannot be made fails the test")]
    fn fresh(test: &str) -> PathBuf {
        crate::testing::run_folder("scratch_names", &format!("pg-stored-case-{test}")).unwrap()
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
            crate::testing::skip("this test folder's file system is case-insensitive: the two names are one folder");
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }
        assert_eq!(real_path(&base.join("pics")).unwrap().file_name(), Some(std::ffi::OsStr::new("pics")));
        assert_eq!(real_path(&base.join("Pics")).unwrap().file_name(), Some(std::ffi::OsStr::new("Pics")));
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A folder's entries for `pick_stored`: each name with its (device, inode), and a count of the identity lookups made.
    fn listing<'a>(
        entries: &'a [(&'a str, (u64, u64))],
        lookups: &'a std::cell::Cell<usize>,
    ) -> impl Iterator<Item = (std::ffi::OsString, impl FnOnce() -> Option<(u64, u64)> + 'a)> + 'a {
        entries.iter().map(move |&(name, identity)| {
            (std::ffi::OsString::from(name), move || {
                lookups.set(lookups.get() + 1);
                Some(identity)
            })
        })
    }

    #[test]
    fn a_case_insensitive_folder_is_spelled_as_it_is_stored() {
        // On a case-insensitive folder (a casefold ext4 folder, a Windows folder mounted in) the typed `photos` is the
        // folder the listing names `Photos`: same device and inode, so the stored spelling is taken. No such folder is on
        // the test machines, so the choice is tested over a listing.
        let lookups = std::cell::Cell::new(0);
        let entries = [("Albums", (1, 10)), ("Photos", (1, 20)), ("photos.txt", (1, 30))];
        let stored = pick_stored(std::ffi::OsStr::new("photos"), (1, 20), listing(&entries, &lookups));
        assert_eq!(stored, "Photos");
        // Only the name equal to it ignoring case was looked up.
        assert_eq!(lookups.get(), 1);
    }

    #[test]
    fn a_name_equal_ignoring_case_but_another_file_is_not_taken() {
        // `PHOTOS` here is another folder (a case-sensitive file system keeps both): its identity differs, so it is not
        // the typed one's spelling, and the typed name is kept when no entry is.
        let lookups = std::cell::Cell::new(0);
        let entries = [("PHOTOS", (1, 40)), ("photos", (1, 20))];
        assert_eq!(pick_stored(std::ffi::OsStr::new("photos"), (1, 20), listing(&entries, &lookups)), "photos");
        let other = [("PHOTOS", (1, 40))];
        assert_eq!(pick_stored(std::ffi::OsStr::new("photos"), (1, 20), listing(&other, &lookups)), "photos");
    }
}
