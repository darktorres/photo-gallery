//! The types and functions the catalog's modules share: the rows and values `core`, `lifecycle` and `writers` pass
//! between them, the SQL constants they share, and the helpers for a folder's subtree. A leaf: it uses no module of
//! the crate but `error`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rusqlite::types::Type;
use rusqlite::{Connection, Row, params};

use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FolderId, RootId};
use pg_model::media::{Ext, Kind};
use pg_model::name::Name;
use pg_model::notice::{Carried, ERROR_CAP, Notice, tidy};
use pg_model::path::{FilePath, PathArena, PathRef};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::turns::Turns;

use crate::error::Result;

/// The parameters of `under_root!` for `root` under `rules`: the root's bytes, and the bounds of its subtree. A
/// drive's root already ends with a separator, so its subtree starts there.
pub(crate) fn under_params(rules: PathRules, root: &pg_model::rules::Path) -> [Vec<u8>; 3] {
    let sep = rules.sep_byte();
    let bytes = rules.join(root);
    let (lo, hi) = match bytes.split_last() {
        Some((&last, init)) if last == sep => {
            let mut hi = init.to_vec();
            hi.push(sep + 1);
            (bytes.clone(), hi)
        }
        _ => {
            let mut lo = bytes.clone();
            lo.push(sep);
            let mut hi = bytes.clone();
            hi.push(sep + 1);
            (lo, hi)
        }
    };
    [bytes, lo, hi]
}

/// The ids of the folders at `path` or below it (`under_root!`), on `conn` (a transaction's too).
pub(crate) fn folders_under(conn: &Connection, rules: PathRules, path: &RulePath) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(under_root!())?;
    let [p1, p2, p3] = under_params(rules, path);
    Ok(stmt.query_map(params![p1, p2, p3], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

/// The folders a scan did not list: each folder of `skipped` (one it couldn't read, or a reparse folder it did not
/// follow) and every folder below it. The scan saw nothing there, so neither the missing marks nor the pairing judge
/// their rows.
pub(crate) fn skipped_folders(conn: &Connection, rules: PathRules, skipped: &[RulePath]) -> Result<BTreeSet<i64>> {
    let mut folders = BTreeSet::new();
    for path in skipped {
        folders.extend(folders_under(conn, rules, path)?);
    }
    Ok(folders)
}

/// A library folder: the user added it, and a rescan walks it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub id: RootId,
    /// Absolute, with no trailing separator.
    pub path: RulePath,
}

/// A catalog that `Catalog::prepare` moved aside, and why: its schema differed from this
/// build's, or it was not a database. The new catalog is made from this build's schema and takes the old one's rows
/// ([`Carried`], the model's, which the notice of the rebuild carries), so the library stays as it was; the old file is kept beside it, whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebuilt {
    /// Where the old catalog is now (beside the new one, with its `-wal`, `-shm` and `-journal` moved with it).
    pub kept: PathBuf,
    /// The first difference from this build's schema, as [`pg_dbfile::Difference`] words it.
    pub why: String,
    /// What the new catalog took from the old one (with their ids).
    pub carried: Carried,
}

impl Rebuilt {
    /// The notice that says so.
    pub fn notice(&self) -> Notice {
        Notice::CatalogRebuilt {
            kept: self.kept.display().to_string(),
            why: tidy(&self.why, ERROR_CAP),
            carried: self.carried.clone(),
        }
    }
}
/// The `meta.path_rules` value of a rule set.
/// The kind in column `i` (`files.kind`): an error for one that doesn't read, as an unknown extension is.
pub(crate) fn kind_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Kind> {
    let key = r.get_ref(i)?.as_str()?;
    Kind::from_key(key).ok_or_else(|| rusqlite::Error::InvalidColumnType(i, format!("kind {key}"), Type::Text))
}

/// The extension in column `i` (`files.ext`): an error for one the library doesn't hold.
pub(crate) fn ext_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Ext> {
    let ext = r.get_ref(i)?.as_str()?;
    Ext::of(ext).ok_or_else(|| rusqlite::Error::InvalidColumnType(i, format!("ext {ext}"), Type::Text))
}

/// The analyzed format in column `i` (`analysis.format`): `None` when there is none (no analysis, or a failed one), and
/// an error for a name that doesn't read.
pub(crate) fn format_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<Format>> {
    let Some(name) = r.get_ref(i)?.as_str_or_null()? else {
        return Ok(None);
    };
    Format::from_name(name)
        .map(Some)
        .ok_or_else(|| rusqlite::Error::InvalidColumnType(i, format!("format {name}"), Type::Text))
}

/// What a queued write needs of its file ([`crate::Catalog::write_target`]): its path, its analyzed format (`None`
/// before the import, or when it failed), and its kind.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteTarget {
    pub path: RulePath,
    pub format: Option<Format>,
    pub kind: Kind,
}

pub(crate) fn rules_name(rules: PathRules) -> &'static str {
    match rules {
        PathRules::Windows => "windows",
        PathRules::Posix => "posix",
    }
}

/// The versions of what the build makes, which `files.pending`, `analysis.borders_due` and the failure ledger are kept
/// against (`analyze::IMPORT_VERSION`, `borders::BORDERS_VERSION`, `pg_render::THUMBS_VERSION`). The library passes them
/// to `Catalog::prepare`: the catalog doesn't depend on the analysis or the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions {
    pub import: i64,
    pub borders: i64,
    pub thumbs: i64,
}

/// The local zone's UTC offsets, in seconds, on January 1 and July 1 of 2000 and of 2030: the zone's fingerprint, as
/// `a,b,c,d`. A UTC-sourced `analysis.taken` depends on the zone only through these offsets, so a change to any of
/// them is a change of zone (or of its daylight-saving rules as they apply to these dates).
pub(crate) const TAKEN_ZONE_QUERY: &str = "SELECT (unixepoch('2000-01-01', 'localtime') - unixepoch('2000-01-01')) || ',' || \
    (unixepoch('2000-07-01', 'localtime') - unixepoch('2000-07-01')) || ',' || \
    (unixepoch('2030-01-01', 'localtime') - unixepoch('2030-01-01')) || ',' || \
    (unixepoch('2030-07-01', 'localtime') - unixepoch('2030-07-01'))";

/// A folder's path by its id.
pub(crate) const FOLDER_PATH: &str = "SELECT path FROM folders WHERE id = ?1";

/// The path of the folder of row `r` (its column 1), read with `folder` ([`FOLDER_PATH`]) into `arena`.
pub(crate) fn folder_path(
    folder: &mut rusqlite::Statement,
    r: &rusqlite::Row,
    arena: &mut PathArena,
) -> rusqlite::Result<PathRef> {
    let id: i64 = r.get(1)?;
    folder.query_row([id], |f| Ok(arena.push(f.get_ref(0)?.as_blob()?)))
}

/// A file as the scanner saw it.
pub struct ScannedFile<'a> {
    pub identity: FileIdentity,
    pub name: &'a Name,
    /// Its last extension (`Ext::of_name`), which also says its kind.
    pub ext: Ext,
    pub size: u64,
    pub mtime: i64,
    /// Its creation time (FILETIME ticks), where the listing gives one (S-17.7 decision 2).
    pub created: Option<i64>,
}

/// A folder's row, with its part of its files' order key (`PathRules::folder_key`).
pub struct Folder {
    pub id: FolderId,
    pub key: Vec<u8>,
}

/// A file the import (or the borders pass) still has to analyze.
#[derive(Clone, Copy, Debug)]
pub struct WorkItem {
    pub id: FileId,
    /// In its list.
    pub path: FilePath,
    pub size: i64,
    pub mtime: i64,
    /// Catalog rotation (`files.rotation`), applied when decoding.
    pub rotation: Turns,
    pub kind: Kind,
}

impl WorkItem {
    /// The file as it was listed, to check against before what was read from it is kept ([`Catalog::is_current`]).
    ///
    /// [`Catalog::is_current`]: crate::Catalog::is_current
    pub fn listed(&self) -> Listed {
        Listed { size: self.size, mtime: self.mtime, rotation: self.rotation }
    }
}

/// A file as a pass listed it: what the pass read must still be the file's for it to be kept (`Catalog::is_current`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listed {
    pub size: i64,
    pub mtime: i64,
    /// The catalog's rotation, which the pass applied.
    pub rotation: Turns,
}

/// A file with borders to crop off (`Catalog::crop_source`).
pub struct CropSource {
    /// In the arena `crop_source` was given.
    pub path: PathRef,
    /// The file as analyzed.
    pub size: i64,
    pub mtime: i64,
    pub format: Format,
    /// The stored size.
    pub width: u32,
    pub height: u32,
    /// Left, top, right, bottom, stored pixels.
    pub cut: [u32; 4],
}

pub struct Status {
    pub files: i64,
    pub missing: i64,
    pub analyzed: i64,
    pub failed: i64,
    pub pending: i64,
    pub pdq_pairs: i64,
    /// Ratings and turns not yet written into the files, and those that keep failing.
    pub queued_writes: i64,
    pub failed_writes: i64,
}

pub(crate) const DELETE_SIDES: &str = "DELETE FROM border_sides WHERE file_id = ?1";
/// The user's rating of a file (`ratings`), set (`?1` the file, `?2` the rating) and cleared.
pub(crate) const SET_RATING: &str = "INSERT INTO ratings(file_id, rating, updated) VALUES (?1, ?2, unixepoch())
     ON CONFLICT(file_id) DO UPDATE SET rating = excluded.rating, updated = excluded.updated";
pub(crate) const DELETE_RATING: &str = "DELETE FROM ratings WHERE file_id = ?1";
pub(crate) const SET_FILE_RATING: &str = "INSERT INTO file_ratings(file_id, value) VALUES (?1, ?2)
     ON CONFLICT(file_id) DO UPDATE SET value = excluded.value";
pub(crate) const DELETE_FILE_RATING: &str = "DELETE FROM file_ratings WHERE file_id = ?1";

/// What recording a file did (`ScanWriter::upsert_file`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Recorded as it was: only marked seen by this scan.
    Same,
    /// A new file, moved, renamed, rewritten, back from missing, or the file that now has an identity another row had.
    Changed,
    /// The identity is a row of another root, which this scan doesn't write (S-17.7 decision 5).
    Foreign,
}

/// The row a file's identity has, as the scan needs it.
pub(crate) struct Hit {
    pub(crate) id: i64,
    pub(crate) folder_id: i64,
    /// The same folder, name, kind, size, mtime, order key, and not missing: nothing to write but the scan's mark.
    pub(crate) same: bool,
    pub(crate) created: Option<i64>,
}
