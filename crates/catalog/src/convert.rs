//! What a migration's data conversion reads from (`schema::Step::Rust`): the old shape of the rows, which is either the file
//! being migrated ([`Source::Main`]) or the old file a rebuild's copy attaches ([`Source::Carried`]). The conversions
//! themselves were removed with the migrations they served (the baseline is the schema now, before the first release).
//! A later conversion finds what it converts by its shape (a column's presence or type), never by the version, and is
//! idempotent, so the same code serves both paths.
//!
//! A leaf: it uses only `rusqlite`.

use rusqlite::Connection;

/// Where a step reads the rows it converts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The file being migrated: its old shape is still there beside the new one, until the step after this one.
    Main,
    /// The old file a rebuild copies from, attached as `carried`; the shared columns are already in `main`.
    Carried,
}

/// The column names of `table` in `schema` (`main` or an attached database); none when the table isn't there.
pub(crate) fn table_columns(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT name FROM pragma_table_info(?1, ?2)")?.query_map([table, schema], |r| r.get(0))?.collect()
}
