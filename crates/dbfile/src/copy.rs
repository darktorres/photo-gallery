//! Opening a copy of a database file, for a bench or a test. A copy made from a built file is not in WAL mode, while the
//! code measured runs in it, so the mode is set here. The owners of live files keep their own policies (`lib.rs`).

use std::path::Path;

use rusqlite::Connection;

/// Opens `path`, a copy of a database file (never a live one), and sets it to WAL mode. Named for the copy it is for:
/// the catalog and the thumbnail cache open their own files with their own settings.
pub fn open_copy_wal(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    Ok(conn)
}
