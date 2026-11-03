//! What a reader checks a file against: its state (`FileState`), and what a thumbnail or the viewer needs of it
//! (`FileSource`, `Placeholder`).

use crate::Catalog;
use crate::Result;
use rusqlite::OptionalExtension;
use rusqlite::types::ValueRef;

use pg_model::analysis::ThumbHash;
use pg_model::ids::FileId;
use pg_model::media::Kind;
use pg_model::turns::Turns;

/// What a file's images are made from, as the catalog has it now. What is made from the file (a thumbnail, a
/// decode, an analysis) is kept under the state it was read in, checked before and after the read
/// ([`FileState::writing`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileState {
    /// With `rotation`, what the thumbnail cache keeps a file's thumbnails under (`thumbs::Source`).
    pub mtime: i64,
    pub rotation: Turns,
    /// The file's size (`files.size`): with `mtime`, what the scanner decides a file changed by.
    pub size: i64,
    /// Quarter turns clockwise asked for and not yet written (`queue_rotation`): what is made from the file is shown
    /// turned by them.
    pub turns: Turns,
    /// A write of the app's into the file is in progress (`file_writes.temp`, until the transaction that records
    /// it): the file may be part-written, or hold a write the catalog doesn't show yet.
    pub writing: bool,
}

/// [`FileState`]'s columns, `f` the file.
macro_rules! file_state_columns {
    () => {
        "f.mtime, f.rotation,
        coalesce((SELECT value FROM file_writes WHERE file_id = f.id AND kind = 'rotation'), 0),
        EXISTS(SELECT 1 FROM file_writes WHERE file_id = f.id AND temp IS NOT NULL),
        f.size"
    };
}

fn file_state_at(r: &rusqlite::Row, first: usize) -> rusqlite::Result<FileState> {
    Ok(FileState {
        mtime: r.get(first)?,
        rotation: Turns::new(r.get(first + 1)?),
        turns: Turns::new(r.get(first + 2)?),
        writing: r.get(first + 3)?,
        size: r.get(first + 4)?,
    })
}

/// The file's state now (`None`: no such file).
pub fn file_state(catalog: &Catalog, file_id: FileId) -> Result<Option<FileState>> {
    let conn = &catalog.conn;
    Ok(conn
        .prepare_cached(concat!("SELECT ", file_state_columns!(), " FROM files f WHERE f.id = ?1"))?
        .query_row([file_id.get()], |r| file_state_at(r, 0))
        .optional()?)
}

/// What the image providers need to make a file's images.
#[derive(Clone, PartialEq, Debug)]
pub struct FileSource {
    pub path: String,
    pub state: FileState,
    /// For a video, the time of the frame that stands for it (`video::poster_ms`; 0 before the import).
    pub poster_ms: Option<i64>,
}

pub fn file_source(catalog: &Catalog, file_id: FileId) -> Result<Option<FileSource>> {
    let conn = &catalog.conn;
    Ok(conn
        .prepare_cached(concat!(
            "SELECT fo.path || '\\' || f.name, f.kind, a.poster_ms, ",
            file_state_columns!(),
            " FROM files f JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
             WHERE f.id = ?1"
        ))?
        .query_row([file_id.get()], |r| {
            let video = r.get_ref(1)?.as_str()? == Kind::Video.key();
            Ok(FileSource {
                path: r.get(0)?,
                state: file_state_at(r, 3)?,
                poster_ms: if video { Some(r.get::<_, Option<i64>>(2)?.unwrap_or(0)) } else { None },
            })
        })
        .optional()?)
}

/// A file's ThumbHash placeholder, and the quarter turns clockwise that show it as the file is shown now: the
/// turns asked for and not yet written, and those kept in the catalog since the analysis that made the hash.
pub struct Placeholder {
    /// `None` before the import.
    pub hash: Option<ThumbHash>,
    pub turns: Turns,
}

pub fn placeholder(catalog: &Catalog, file_id: FileId) -> Result<Option<Placeholder>> {
    let conn = &catalog.conn;
    Ok(conn
        .prepare_cached(concat!(
            "SELECT a.thumbhash, (f.rotation - coalesce(a.rotation, f.rotation) + 4) % 4, ",
            file_state_columns!(),
            " FROM files f LEFT JOIN analysis a ON a.file_id = f.id WHERE f.id = ?1"
        ))?
        .query_row([file_id.get()], |r| {
            let kept: u8 = r.get(1)?;
            // Read in place: the blob isn't copied into a `Vec` first.
            let hash = match r.get_ref(0)? {
                ValueRef::Null => None,
                v => ThumbHash::new(v.as_blob()?),
            };
            Ok(Placeholder { hash, turns: Turns::new(kept).plus(file_state_at(r, 2)?.turns) })
        })
        .optional()?)
}
