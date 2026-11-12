//! What a database file is before anything is written to it: new, this build's, of another layout, another program's,
//! or damaged. Reading only: a file that is not the owner's gets no pragma and no write. The owner decides what to do
//! with what it finds (the catalog migrates or rebuilds, the cache starts over), and may look closer at a file found
//! [`Found::Differs`] (an earlier layout's version, the rows it keeps).

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::Connection;

use crate::schema::{Difference, Schema, application_id};

/// Why SQLite finds a file damaged: not a database at all, or a database whose pages don't hold together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    /// `NotADatabase`: the file's header is not SQLite's.
    NotADatabase,
    /// `DatabaseCorrupt`: SQLite's account of what doesn't hold together.
    Corrupt(String),
}

impl Damage {
    /// The damage `e` reports, if its code is one of SQLite's for that. Any other failure (busy, no permission, a full
    /// disk) is not the file's fault: `None`, and it stays an error.
    pub fn of(e: &rusqlite::Error) -> Option<Damage> {
        match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::NotADatabase) => Some(Damage::NotADatabase),
            Some(rusqlite::ErrorCode::DatabaseCorrupt) => Some(Damage::Corrupt(e.to_string())),
            _ => None,
        }
    }

    /// [`Damage::of`] for the first SQLite error in `e`'s chain.
    pub fn in_chain(e: &(dyn std::error::Error + 'static)) -> Option<Damage> {
        let mut cause = Some(e);
        while let Some(e) = cause {
            if let Some(found) = e.downcast_ref::<rusqlite::Error>().and_then(Damage::of) {
                return Some(found);
            }
            cause = e.source();
        }
        None
    }
}

impl std::fmt::Display for Damage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Damage::NotADatabase => write!(f, "it is not a database"),
            Damage::Corrupt(why) => write!(f, "it is damaged ({why})"),
        }
    }
}

/// The schema a build makes, built once, lazily, in memory: `make_layout` makes the layout in an empty database, and
/// `seeds` are the `(table, column)` whose keys are compared ([`Schema::read`]). The owner makes its files with the same
/// `make_layout`, so the file and the expectation can't disagree.
pub struct Expected {
    make_layout: fn(&Connection) -> rusqlite::Result<()>,
    seeds: &'static [(&'static str, &'static str)],
    schema: OnceLock<Schema>,
}

impl Expected {
    pub const fn new(
        make_layout: fn(&Connection) -> rusqlite::Result<()>,
        seeds: &'static [(&'static str, &'static str)],
    ) -> Expected {
        Expected { make_layout, seeds, schema: OnceLock::new() }
    }

    /// The schema, made the first time it is asked for.
    pub fn schema(&self) -> rusqlite::Result<&Schema> {
        if let Some(schema) = self.schema.get() {
            return Ok(schema);
        }
        let conn = Connection::open_in_memory()?;
        (self.make_layout)(&conn)?;
        let schema = Schema::read(&conn, self.seeds)?;
        Ok(self.schema.get_or_init(|| schema))
    }

    /// The keys compared with the file's.
    pub fn seeds(&self) -> &'static [(&'static str, &'static str)] {
        self.seeds
    }
}

/// What a file is, against what its owner makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// No file, or one with no objects.
    New,
    /// The owner's layout, as this build makes it.
    Usable,
    /// Another layout, with the owner's application id or none (an earlier build that set none): the first difference.
    Differs(Difference),
    /// A database another program marked as its own.
    Foreign { application_id: i64 },
    /// Not a database, or a damaged one.
    Damaged(Damage),
}

/// What the file at `path` (opened by `open`, which gives the form SQLite takes) is, against `expected`, for an owner
/// whose files carry `application_id`. A read SQLite fails as damage is [`Found::Damaged`]; any other failure is an
/// error.
pub fn judge(
    path: &Path,
    open: impl FnOnce(&Path) -> rusqlite::Result<Connection>,
    expected: &Expected,
    owner: i64,
) -> rusqlite::Result<Found> {
    if !path.exists() {
        return Ok(Found::New);
    }
    let want = expected.schema()?;
    let conn = open(path)?;
    match judge_conn(&conn, expected.seeds(), want, owner) {
        Ok(found) => Ok(found),
        Err(e) => Damage::of(&e).map(Found::Damaged).ok_or(e),
    }
}

/// [`judge`]'s reads, on an open file.
pub fn judge_conn(conn: &Connection, seeds: &[(&str, &str)], want: &Schema, owner: i64) -> rusqlite::Result<Found> {
    let found = Schema::read(conn, seeds)?;
    if found.is_empty() {
        return Ok(Found::New);
    }
    let id = application_id(conn)?;
    if id != owner && id != 0 {
        return Ok(Found::Foreign { application_id: id });
    }
    Ok(match found.difference(want) {
        Some(difference) => Found::Differs(difference),
        // The layout, unmarked: an earlier build's that set no id. Not this build's file until it is marked.
        None if id != owner => {
            Found::Differs(Difference::Changed { kind: "header".into(), name: "application_id".into() })
        }
        None => Found::Usable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: i64 = 0x5445_5354;

    fn layout(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch("CREATE TABLE t(a INTEGER);")?;
        conn.pragma_update(None, "application_id", ID)
    }

    static EXPECTED: Expected = Expected::new(layout, &[]);

    fn found(sql: &str) -> Found {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(sql).unwrap();
        judge_conn(&conn, &[], EXPECTED.schema().unwrap(), ID).unwrap()
    }

    #[test]
    fn a_file_is_new_usable_another_layout_or_another_programs() {
        assert_eq!(found(""), Found::New);
        assert_eq!(found(&format!("CREATE TABLE t(a INTEGER); PRAGMA application_id = {ID};")), Found::Usable);
        assert!(matches!(found(&format!("CREATE TABLE t(b TEXT); PRAGMA application_id = {ID};")), Found::Differs(_)));
        assert!(matches!(found("CREATE TABLE t(a INTEGER);"), Found::Differs(_)), "unmarked");
        assert_eq!(
            found("CREATE TABLE t(a INTEGER); PRAGMA application_id = 7;"),
            Found::Foreign { application_id: 7 }
        );
    }

    #[test]
    fn a_file_that_is_not_a_database_is_damaged() {
        let dir = pg_testkit::run_folder("scratch_dbfile", "pg-dbfile-judge").unwrap();
        let path = dir.join("not-a-db.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let judged = judge(&path, |p| Connection::open(p), &EXPECTED, ID).unwrap();
        assert_eq!(judged, Found::Damaged(Damage::NotADatabase));
        assert_eq!(judge(&dir.join("absent.sqlite"), |p| Connection::open(p), &EXPECTED, ID).unwrap(), Found::New);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
