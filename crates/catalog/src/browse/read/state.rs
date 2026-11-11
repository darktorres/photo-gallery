//! The readers of a file's state (`pg_model::state`): the state now (`file_state`), what the image providers need
//! (`file_source`), and the ThumbHash placeholder (`placeholder`).

use crate::core::Catalog;
use crate::error::Result;
use rusqlite::OptionalExtension;
use rusqlite::types::ValueRef;

use pg_model::analysis::ThumbHash;
use pg_model::ids::FileId;
use pg_model::media::Kind;
use pg_model::state::{FileSource, FileState, Placeholder};
use pg_model::turns::Turns;

/// [`FileState`]'s columns, `f` the file.
macro_rules! file_state_columns {
    () => {
        concat!(
            "f.mtime, f.rotation,
        coalesce((SELECT value FROM file_writes WHERE file_id = f.id AND kind = '",
            pg_model::write_rotation!(),
            "'), 0),
        EXISTS(SELECT 1 FROM file_writes WHERE file_id = f.id AND temp IS NOT NULL),
        f.size"
        )
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

impl Catalog {
    /// The file's state now (`None`: no such file).
    pub fn file_state(&self, file_id: FileId) -> Result<Option<FileState>> {
        let conn = &self.conn;
        Ok(conn
            .prepare_cached(concat!("SELECT ", file_state_columns!(), " FROM files f WHERE f.id = ?1"))?
            .query_row([file_id.get()], |r| file_state_at(r, 0))
            .optional()?)
    }

    pub fn file_source(&self, file_id: FileId) -> Result<Option<FileSource>> {
        let conn = &self.conn;
        let row = conn
            .prepare_cached(concat!(
                "SELECT fo.path, f.name, f.kind, a.poster_ms, ",
                file_state_columns!(),
                " FROM files f JOIN folders fo ON fo.id = f.folder_id LEFT JOIN analysis a ON a.file_id = f.id
                 WHERE f.id = ?1"
            ))?
            .query_row([file_id.get()], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    crate::types::kind_at(r, 2)?,
                    r.get::<_, Option<i64>>(3)?,
                    file_state_at(r, 4)?,
                ))
            })
            .optional()?;
        Ok(match row {
            Some((folder, name, kind, poster_ms, state)) => {
                let video = kind == Kind::Video;
                Some(FileSource {
                    path: self.file_path_of(&folder, &name)?,
                    state,
                    poster_ms: if video { Some(poster_ms.unwrap_or(0)) } else { None },
                })
            }
            None => None,
        })
    }

    pub fn placeholder(&self, file_id: FileId) -> Result<Option<Placeholder>> {
        let conn = &self.conn;
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
}
