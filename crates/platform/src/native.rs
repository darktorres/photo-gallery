//! Paths and names as the system has them. A unit is a UTF-16 unit on Windows and a byte elsewhere, and nothing here
//! converts lossily: a name the system holds (an unpaired surrogate on NTFS, any bytes on Unix) reaches the system,
//! and the library, unchanged.
//!
//! A [`NativePath`] is what a direct Win32 call is given (the scanner's `CreateFileW`). On Windows it is NUL-terminated,
//! and a plain absolute path (`D:\…` or `\\server\share\…`, with no `/` and no `.` or `..` part) is given in the
//! verbatim form (`\\?\`, `\\?\UNC\`), whatever its length. The verbatim form is the one rule: Win32 takes it as it
//! is, where the plain form is normalized first, which drops a part's trailing dot, and the trailing dots and spaces
//! of the path's end (NTFS holds such a name, as in a folder `Acme Inc.` copied from Linux or macOS, and the plain
//! form then names `Acme Inc`, another folder or none). A stored path reaches std, FFmpeg and ExifTool through
//! `names::disk_path`, its verbatim form: the one way production code turns a catalog's path into one it opens
//! (MODULARITY_REVIEW_4 R-2). The volume calls (`GetVolumePathNameW`, `GetDriveTypeW`) take the plain form
//! ([`NativePath::set_plain`]) as they always did.
//!
//! ExifTool reads and writes a verbatim path, through its stay-open argument file and its command line alike (checked
//! with ExifTool 13.60, in a folder `Fotos ä.` beside `Fotos ä`). Two readers normalize every name they are given, the
//! verbatim one too: Qt's file engine (`GetFullPathNameW`, in `QFSFileEnginePrivate::longFileName`), so the video
//! player is given the file open (`file::player_handle`); and the shell's parse, so a folder dialog starts at the
//! nearest folder whose names survive it ([`check_folder_names`], `picker::start_folder`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use pg_model::rules::PathRules;

/// The path rules of this platform, the ones its catalogs are made under: [`PathRules::Windows`] on Windows,
/// [`PathRules::Posix`] elsewhere. The model holds both rule sets as data; which one this machine's paths follow is
/// a fact of the platform, so it is stated here.
pub const PATH_RULES: PathRules = if cfg!(windows) { PathRules::Windows } else { PathRules::Posix };

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

/// A path as a system call takes it: its units joined, with a NUL on Windows (and the verbatim form when it is a plain
/// absolute path). Its allocation is kept from one path to the next, so a pass builds its paths in one buffer.
#[derive(Default, Debug)]
pub struct NativePath(Vec<Unit>);

impl NativePath {
    /// `parts` concatenated into this path (the first starts it), replacing what it held: on Windows in the verbatim
    /// form when the whole path is a plain absolute one ([`verbatim`]'s rule).
    pub fn set(&mut self, parts: &[&[Unit]]) {
        self.0.clear();
        set_units(&mut self.0, parts, true);
    }

    /// `parts` concatenated as they are, never in the verbatim form: for the volume calls, which are given the path
    /// as it was typed.
    pub fn set_plain(&mut self, parts: &[&[Unit]]) {
        self.0.clear();
        set_units(&mut self.0, parts, false);
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

/// `path` for std's own calls, so they reach the file it names exactly: on Windows, the verbatim form of a plain
/// absolute path (`D:\…` to `\\?\D:\…`, `\\server\share\…` to `\\?\UNC\server\share\…`) with no `/` and no `.` or
/// `..` part; any other path (relative, on a drive without its root, already verbatim, a device path) as it is.
/// Elsewhere, the path as it is.
pub fn verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
        to_verbatim(&mut units);
        PathBuf::from(std::ffi::OsString::from_wide(&units))
    }
    #[cfg(unix)]
    {
        path.to_path_buf()
    }
}

/// A folder of a path whose name Win32 may change in the plain form of the path: it ends in a dot or a space, which
/// the normalization drops (a dot always, a space at the path's end), so the plain path names another folder (or
/// none). The shell normalizes every path it parses, the verbatim one too (it refuses that form), so a folder dialog
/// can't start in such a folder (`picker::start_folder`). A space counts wherever it is, as a path that ends at the
/// folder has it dropped too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlteredFolder {
    /// The path refused.
    pub path: PathBuf,
    /// The folder's name, as the system holds it.
    pub folder: std::ffi::OsString,
}

impl std::fmt::Display for AlteredFolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: its folder {:?} ends in a dot or a space, which Windows drops from a path it normalizes",
            self.path.display(),
            self.folder.to_string_lossy()
        )
    }
}

impl std::error::Error for AlteredFolder {}

/// Whether every folder of `path` (each part before its last) reaches the folder it names in the plain form: an
/// [`AlteredFolder`] for the first that ends in a dot or a space (the parts `.` and `..` are not names, and pass).
/// Always `Ok` off Windows, where no path is normalized.
pub fn check_folder_names(path: &Path) -> Result<(), AlteredFolder> {
    #[cfg(windows)]
    {
        let Some(parent) = path.parent() else { return Ok(()) };
        for part in parent.components() {
            if let std::path::Component::Normal(name) = part
                && altered(name)
            {
                return Err(AlteredFolder { path: path.to_path_buf(), folder: name.to_os_string() });
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        let _ = path;
        Ok(())
    }
}

/// A name the Win32 normalization may change: one that ends in a dot or a space.
#[cfg_attr(unix, expect(dead_code, reason = "no path is normalized off Windows"))]
fn altered(name: &OsStr) -> bool {
    name.as_encoded_bytes().last().is_some_and(|&b| b == b'.' || b == b' ')
}

#[cfg(windows)]
fn set_units(out: &mut Vec<Unit>, parts: &[&[Unit]], verbatim: bool) {
    for part in parts {
        out.extend_from_slice(part);
    }
    if verbatim {
        to_verbatim(out);
    }
    out.push(0);
}

#[cfg(unix)]
fn set_units(out: &mut Vec<Unit>, parts: &[&[Unit]], _verbatim: bool) {
    for part in parts {
        out.extend_from_slice(part);
    }
}

/// The units of the verbatim prefix, and of a device path (`\\.\`), which is left as it is.
#[cfg(windows)]
const VERBATIM: &[Unit] = &[0x5C, 0x5C, 0x3F, 0x5C]; // \\?\
/// What the verbatim form of a share puts after its leading `\\`: `\\server` becomes `\\?\UNC\server`.
#[cfg(windows)]
const UNC_AFTER: &[Unit] = &[
    0x3F, // ?
    0x5C, // \
    0x55, // U
    0x4E, // N
    0x43, // C
    0x5C, // \
];
#[cfg(windows)]
const DEVICE: &[Unit] = &[0x5C, 0x5C, 0x2E, 0x5C]; // \\.\

/// How a path is given in the verbatim form.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// As it is: not a plain absolute path.
    Plain,
    /// `D:\…`: `\\?\` goes in front.
    Drive,
    /// `\\server\share\…`: its leading `\\` becomes `\\?\UNC\`.
    Unc,
}

/// The form `path` (its units, no NUL) takes: verbatim only for a plain absolute path whose parts are all names. A
/// `/` (the verbatim form doesn't take it as a separator), a `.` or `..` part, or an empty part before the last (all
/// of which the plain form resolves, and the verbatim form would take literally) keep it plain.
#[cfg(windows)]
fn form_of(path: &[Unit]) -> Form {
    const SLASH: Unit = 0x2F;
    const DOT: Unit = 0x2E;
    const COLON: Unit = 0x3A;
    if path.contains(&SLASH) || path.starts_with(VERBATIM) || path.starts_with(DEVICE) {
        return Form::Plain;
    }
    let (form, rest) = if let Some(rest) = path.strip_prefix(&[SEP, SEP][..]) {
        if rest.is_empty() {
            return Form::Plain;
        }
        (Form::Unc, rest)
    } else if let [letter, COLON, SEP, rest @ ..] = path
        && u8::try_from(*letter).is_ok_and(|b| b.is_ascii_alphabetic())
    {
        (Form::Drive, rest)
    } else {
        return Form::Plain;
    };
    let mut parts = rest.split(|&u| u == SEP).peekable();
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        if (part.is_empty() && !last) || part == [DOT] || part == [DOT, DOT] {
            return Form::Plain;
        }
    }
    form
}

/// `path` (its units, no NUL) put in its verbatim form, in place, where [`form_of`] says it takes one.
#[cfg(windows)]
fn to_verbatim(path: &mut Vec<Unit>) {
    match form_of(path) {
        Form::Plain => {}
        Form::Drive => {
            path.extend_from_slice(VERBATIM);
            path.rotate_right(VERBATIM.len());
        }
        Form::Unc => {
            path.extend_from_slice(UNC_AFTER);
            if let Some(after) = path.get_mut(2..) {
                after.rotate_right(UNC_AFTER.len());
            }
        }
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
    fn absolute_paths_are_verbatim_whatever_their_length() {
        let mut path = NativePath::default();
        path.set(&[&wide(r"D:\a")[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(r"\\?\D:\a\b.jpg"));
        // The drive root keeps its separator.
        path.set(&[&wide("D:")[..], &[SEP][..]]);
        assert_eq!(path.units(), nul(r"\\?\D:\"));
        let dir = "x".repeat(250);
        path.set(&[&wide(&format!(r"D:\{dir}"))[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(&format!(r"\\?\D:\{dir}\b.jpg")));
        path.set(&[&wide(r"\\nas\photos")[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(r"\\?\UNC\nas\photos\b.jpg"));
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
    fn the_whole_path_decides_its_form() {
        // The first part alone is not a path the rule can judge: the joined units are.
        let mut path = NativePath::default();
        path.set(&[&wide("D:")[..], &wide(r"\a")[..], &[SEP][..], &wide("b.jpg")[..]]);
        assert_eq!(path.units(), nul(r"\\?\D:\a\b.jpg"));
        path.set(&[&wide(r"D:\a")[..], &wide("/b.jpg")[..]]);
        assert_eq!(path.units(), nul(r"D:\a/b.jpg"), "a slash anywhere keeps it plain");
    }

    #[cfg(windows)]
    #[test]
    fn other_paths_are_used_as_they_are() {
        let mut path = NativePath::default();
        let plain = [r"a\b.jpg", r"D:a\b.jpg", r"\a\b.jpg", r"D:\a/b.jpg", r"D:\a\..\b.jpg", r"D:\a\.\b.jpg"];
        for plain in plain.into_iter().chain([r"D:\a\\b"]) {
            path.set(&[&wide(plain)[..]]);
            assert_eq!(path.units(), nul(plain), "{plain}");
        }
        // The volume calls are given the path as it is.
        path.set_plain(&[&wide(r"D:\a")[..], &[SEP][..]]);
        assert_eq!(path.units(), nul(r"D:\a\"));
    }

    #[cfg(windows)]
    #[test]
    fn std_paths_take_the_same_rule() {
        assert_eq!(verbatim(Path::new(r"D:\Acme Inc.\a.jpg")), Path::new(r"\\?\D:\Acme Inc.\a.jpg"));
        assert_eq!(verbatim(Path::new(r"\\nas\photos\a.jpg")), Path::new(r"\\?\UNC\nas\photos\a.jpg"));
        assert_eq!(verbatim(Path::new(r"a\b.jpg")), Path::new(r"a\b.jpg"));
        assert_eq!(verbatim(Path::new("D:/a/b.jpg")), Path::new("D:/a/b.jpg"));
    }

    /// A folder whose name ends in a dot is the folder it names in the verbatim form, and not its sibling without the
    /// dot, which the plain form reaches.
    #[cfg(windows)]
    #[test]
    fn a_folder_with_a_trailing_dot_is_reached_verbatim() {
        let dir = std::path::absolute(pg_testkit::test_folder("scratch_native", "trailing-dot").unwrap()).unwrap();
        let dotted = dir.join("Acme Inc.");
        let sibling = dir.join("Acme Inc");
        std::fs::create_dir_all(verbatim(&dotted)).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(verbatim(&dotted.join("a.jpg")), b"dotted").unwrap();
        std::fs::write(sibling.join("a.jpg"), b"sibling").unwrap();
        assert_eq!(std::fs::read(verbatim(&dotted.join("a.jpg"))).unwrap(), b"dotted");
        assert_eq!(std::fs::read(dotted.join("a.jpg")).unwrap(), b"sibling", "the plain form drops the dot");
        let mut native = NativePath::default();
        set_os_str(&mut native, dotted.join("a.jpg").as_os_str());
        let file = crate::file::open_read(&native).unwrap();
        assert_eq!(std::io::read_to_string(file).unwrap(), "dotted");
        std::fs::remove_dir_all(verbatim(&dir)).unwrap();
    }

    #[test]
    fn a_folder_ending_in_a_dot_or_a_space_is_refused_on_windows() {
        let refused = check_folder_names(Path::new(r"D:\Acme Inc.\a.jpg"));
        let spaced = check_folder_names(Path::new(r"D:\a \b\c.jpg"));
        if cfg!(windows) {
            assert_eq!(refused.unwrap_err().folder, "Acme Inc.");
            assert_eq!(spaced.unwrap_err().folder, "a ");
        } else {
            assert!(refused.is_ok() && spaced.is_ok(), "no path is normalized off Windows");
        }
        // The file's own name, and the `.` and `..` parts, are not folders' names.
        assert!(check_folder_names(Path::new(r"D:\a\b.")).is_ok());
        assert!(check_folder_names(Path::new(r"D:\a\..\b.jpg")).is_ok());
        assert!(check_folder_names(Path::new(r"D:\a\b.jpg")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn units_are_the_bytes_as_they_are() {
        let mut path = NativePath::default();
        path.set(&[&b"/a/"[..], &[0xFF, b'b'][..]]);
        assert_eq!(path.units(), b"/a/\xFFb");
        assert_eq!(verbatim(Path::new("/a/b.")), Path::new("/a/b."));
    }
}
