//! The file side of a write: the temp beside it, the check that it is as it was read (by its size and mtime,
//! `pg_platform::file::stat`), and the copy into it in place.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::write::WriteKind;

/// The size and mtime of the file at `path` (its disk path), with the file named in the error.
pub(crate) fn stat(path: &Path) -> Result<(i64, i64)> {
    pg_platform::file::stat(path).with_context(|| format!("reading {}", pg_platform::names::shown(path)))
}

/// The original, opened for the copy (`now`, its size and mtime), is as it was when it was read (`before`): another
/// program's save in between fails the attempt, which counts, and the next one reads the file again.
pub(crate) fn unchanged(shown: &str, before: (i64, i64), now: (i64, i64)) -> Result<()> {
    if before != now {
        bail!("{shown} changed while its change was being prepared: it is read again at the next try");
    }
    Ok(())
}

/// Delete the temp files `temps` (under `rules`) of the writes queued for files a scan replaced (decision 7), by their
/// disk paths: a plain path would miss a temp in a folder whose name ends in a dot, or delete a sibling folder's file of
/// the same name. A temp already gone is no error; the ones that couldn't be deleted, with why.
pub fn discard_temps(rules: PathRules, temps: &[RulePath]) -> Vec<(RulePath, std::io::Error)> {
    temps
        .iter()
        .filter_map(|temp| match std::fs::remove_file(pg_platform::names::disk_path(rules, temp)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Some((temp.clone(), e)),
            _ => None,
        })
        .collect()
}

/// The temp file of a file for a kind of write: beside the original, so it is on the same drive, named by the
/// original's name with a dot in front and the kind and `.pg-write` after it (`.a.jpg.rating.pg-write`; the scanner
/// ignores the extension). A rating and a turn of one file each have theirs, so neither job touches the other's. Its
/// parts are the original's, but the last.
pub fn temp_of(path: &RulePath, kind: WriteKind) -> RulePath {
    let suffix = match kind {
        WriteKind::Rating => ".rating.pg-write",
        WriteKind::Turns => ".turns.pg-write",
    };
    let mut temp = path.clone();
    if let Some(name) = temp.parts.pop() {
        temp.parts.push(name.with_affixes(".", suffix));
    }
    temp
}

/// Flush a file's content to disk (the file at its disk path).
pub(crate) fn sync(path: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Write `data` into the original, open for the copy, in place (same file, same NTFS identity), and flush it. The
/// handle is closed on return, before anything stats the file.
pub(crate) fn write_into(mut file: File, data: &[u8]) -> Result<()> {
    file.write_all(data)?;
    file.set_len(u64::try_from(data.len())?)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pg_model::rules::PathRules;

    use super::*;

    #[test]
    fn a_file_changed_since_it_was_read_is_not_copied_into() {
        assert!(unchanged("a.jpg", (10, 100), (10, 100)).is_ok());
        let changed = unchanged("a.jpg", (10, 100), (12, 100)).unwrap_err().to_string();
        assert!(changed.contains("a.jpg changed while its change was being prepared"), "{changed}");
        assert!(unchanged("a.jpg", (10, 100), (10, 101)).is_err(), "a save of the same size");
    }

    #[test]
    fn each_kind_of_write_has_its_own_temp() {
        let path = PathRules::Windows.parse(r"D:\p\a.jpg").unwrap();
        let rating = temp_of(&path, WriteKind::Rating);
        let turns = temp_of(&path, WriteKind::Turns);
        assert_eq!(PathRules::Windows.display(&rating), r"D:\p\.a.jpg.rating.pg-write");
        assert_eq!(PathRules::Windows.display(&turns), r"D:\p\.a.jpg.turns.pg-write");
    }
}
