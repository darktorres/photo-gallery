//! The catalog's conversions at the SQL edge, one rule per kind. A file identity (`volume`, `file_index`) is the same 64
//! bits as an `i64` in storage, so it converts by its bits ([`identity_to_sql`]). A count or a size
//! is a `u64` on both sides: the workspace's rusqlite is built with `fallible_uint`, so it reads and writes a `u64` with a
//! checked conversion (a negative stored value is an error through `?`, never 0). A count held as a `usize` converts by
//! `pg_model::num::count_u64`, which saturates. The rest of the crate holds these values as their Rust types.
//!
//! It also holds what a migration's data conversion reads from (`schema::Step::Rust`): the old shape of the rows, which is
//! either the file being migrated ([`Source::Main`]) or the old file a rebuild's copy attaches ([`Source::Carried`]). The
//! conversions themselves were removed with the migrations they served (the baseline is the schema now, before the first
//! release). A later conversion finds what it converts by its shape (a column's presence or type), never by the version,
//! and is idempotent, so the same code serves both paths.
//!
//! It uses only `rusqlite`.

use rusqlite::Connection;

/// A file identity (a volume's serial, a file's index) as the `i64` SQLite stores: the same 64 bits, since SQLite's
/// integers are signed. The identity is read back as the same bits (`i64::cast_unsigned`), where a caller needs it.
pub(crate) fn identity_to_sql(identity: u64) -> i64 {
    identity.cast_signed()
}

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
