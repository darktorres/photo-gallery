//! A saved result's file: named after the photo, at the first free name of the folder, written through a
//! temporary so that a failure leaves no partial file at the name.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use pg_platform::file::TempFile;

/// The photo's stem as a saved file's: `stem` kept exactly, then ` (<suffix>)`. The suffix is text the search returns, so
/// the characters Windows refuses in names become `_`.
pub(super) fn saved_stem(stem: &OsStr, suffix: &str) -> OsString {
    let clean: String = suffix.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let mut name = stem.to_os_string();
    name.push(format!(" ({})", clean.trim()));
    name
}

/// How many names [`claim`] tries (`<stem>`, then ` (2)` up to this) before it gives up.
const MAX_CLAIMS: u32 = 10_000;

/// The file name `<stem>.<ext>`, or with ` (n)` after the stem when `n` is past 1.
fn numbered(stem: &OsStr, ext: &str, n: u32) -> OsString {
    let mut name = stem.to_os_string();
    if n > 1 {
        name.push(format!(" ({n})"));
    }
    name.push(format!(".{ext}"));
    name
}

/// Takes the first free name of `folder/<stem>.<ext>`, ` (2)`, ` (3)`… by making it, empty: `create_new` makes the
/// check and the claim one step, so a name another program takes at the same time is not written over.
fn claim(folder: &Path, stem: &OsStr, ext: &str) -> io::Result<PathBuf> {
    for n in 1..=MAX_CLAIMS {
        let path = folder.join(numbered(stem, ext, n));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(ErrorKind::AlreadyExists, format!("{MAX_CLAIMS} names of this file are all taken")))
}

/// The temporary a save writes before it takes the final name: `.<stem>.<ext>.pg-save` beside it (the repository's
/// temporaries are dot-names beside their target: `.pg-write`, `.pg-crop`).
fn temp_path(folder: &Path, stem: &OsStr, ext: &str) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(numbered(stem, ext, 1));
    name.push(".pg-save");
    folder.join(name)
}

/// Writes `bytes` to a new file in `folder` named `<stem>.<ext>` (or ` (2)`, ` (3)`… after the stem when that is
/// taken) and returns its path. The bytes go to a temporary beside it first ([`temp_path`]), synced; then the final
/// name is claimed ([`claim`]) and the temporary renamed over the empty claim. A failure at any step removes both, so
/// a failed write leaves no partial file at the photo's name. The error comes with the path it was at.
///
/// What a crash leaves: during the write, the temporary (its name is fixed, so the next save of that name replaces
/// it); in the instant between the claim and the rename, the temporary and an empty file at the claimed name.
pub(super) fn write_new(folder: &Path, stem: &OsStr, ext: &str, bytes: &[u8]) -> Result<PathBuf, (PathBuf, io::Error)> {
    write_new_with(folder, stem, ext, bytes, |from, to| std::fs::rename(from, to))
}

/// [`write_new`], with the rename that puts the temporary at the claimed name given as `rename` (the seam a test
/// uses to make that step fail, after the claim).
fn write_new_with(
    folder: &Path,
    stem: &OsStr,
    ext: &str,
    bytes: &[u8],
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, (PathBuf, io::Error)> {
    let temp = TempFile::new(&temp_path(folder, stem, ext));
    let at_temp = |error: io::Error| (temp.path().to_path_buf(), error);
    let mut file = File::create(temp.path()).map_err(at_temp)?;
    file.write_all(bytes).map_err(at_temp)?;
    file.sync_all().map_err(at_temp)?;
    drop(file);
    let path = claim(folder, stem, ext).map_err(|error| (folder.join(numbered(stem, ext, 1)), error))?;
    let placeholder = TempFile::new(&path);
    rename(temp.path(), &path).map_err(|error| (path.clone(), error))?;
    placeholder.persist();
    temp.persist();
    Ok(path)
}

/// The file-name stem of the photo, kept exactly: `IMG_1234` for `IMG_1234.jpg`.
pub(super) fn stem_of(photo: &Path) -> OsString {
    photo.file_stem().map_or_else(|| OsString::from("image"), OsStr::to_os_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names in `folder`, sorted.
    fn names_in(folder: &Path) -> Vec<OsString> {
        let mut names: Vec<OsString> =
            std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        names.sort();
        names
    }

    #[test]
    fn saved_names_follow_the_photo() {
        let folder = pg_testkit::run_folder("scratch_app", "yandex-names").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        let path = write_new(&folder, &stem, "jpg", b"first").unwrap();
        assert_eq!(path, folder.join("IMG_1234 (yandex 1920x1080).jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(names_in(&folder), [OsString::from("IMG_1234 (yandex 1920x1080).jpg")], "no temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn two_saves_of_one_name_get_two_files() {
        let folder = pg_testkit::run_folder("scratch_app", "yandex-twice").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        let first = write_new(&folder, &stem, "jpg", b"first").unwrap();
        let second = write_new(&folder, &stem, "jpg", b"second").unwrap();
        assert_eq!(second, folder.join("IMG_1234 (yandex 1920x1080) (2).jpg"));
        assert_eq!(std::fs::read(&first).unwrap(), b"first", "the first is not written over");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        assert_eq!(names_in(&folder).len(), 2, "no temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_file_at_the_name() {
        let folder = pg_testkit::run_folder("scratch_app", "yandex-failed").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        // A folder where the temporary goes: the write to it fails.
        let temp = temp_path(&folder, &stem, "jpg");
        std::fs::create_dir(&temp).unwrap();
        let (at, _) = write_new(&folder, &stem, "jpg", b"bytes").unwrap_err();
        assert_eq!(at, temp, "the error names where it failed");
        assert!(!folder.join("IMG_1234 (yandex 1920x1080).jpg").exists(), "no partial or empty file at the name");
        assert_eq!(names_in(&folder), [temp.file_name().unwrap().to_os_string()], "only the folder put there");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_rename_after_the_claim_leaves_no_file_at_the_name() {
        let folder = pg_testkit::run_folder("scratch_app", "yandex-renamed").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        // The claim (an empty file at the name) is made, then the rename of the temporary onto it is refused.
        let refused = write_new_with(&folder, &stem, "jpg", b"bytes", |_, _| Err(io::Error::other("rename refused")));
        let (at, error) = refused.unwrap_err();
        assert_eq!(at, folder.join("IMG_1234 (yandex 1920x1080).jpg"), "the error names the claimed name");
        assert_eq!(error.to_string(), "rename refused");
        assert!(names_in(&folder).is_empty(), "neither the claim nor the temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_photos_stem_is_kept_exactly_and_only_the_suffix_is_cleaned() {
        let stem = pg_testkit::odd_name("IMG_");
        let mut expected = stem.clone();
        expected.push(" (a_b_c)");
        assert_eq!(saved_stem(&stem, "a/b:c"), expected);
    }
}
