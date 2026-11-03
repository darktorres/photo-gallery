//! The duplicates' reads (tiers 0–1): identical files (`file_hash`), identical pixels (`pixel_hash`), and the PDQ pairs.
//! The counts for a report, and the edges that group files: the grouping itself (union-find) is `pg-dedup`'s.

use anyhow::Result;
use pg_model::ids::FileId;
use rusqlite::Connection;

/// The present files' current analyses.
macro_rules! current {
    () => {
        "FROM analysis a JOIN files f ON f.id = a.file_id WHERE a.error IS NULL AND f.missing = 0"
    };
}

/// The groups of a hash column (`file_hash`, tier 0a; `pixel_hash`, 0b): (how many, how many files in them).
macro_rules! groups {
    ($column:literal) => {
        concat!(
            "SELECT count(*), coalesce(sum(n), 0) FROM
             (SELECT count(*) AS n ",
            current!(),
            " GROUP BY a.",
            $column,
            " HAVING n > 1)"
        )
    };
}

/// The files of those groups, as (hash, file id), by hash.
macro_rules! grouped {
    ($column:literal) => {
        concat!(
            "SELECT a.",
            $column,
            ", a.file_id ",
            current!(),
            " AND a.",
            $column,
            " IN
             (SELECT a.",
            $column,
            " ",
            current!(),
            " GROUP BY a.",
            $column,
            " HAVING count(*) > 1) ORDER BY a.",
            $column
        )
    };
}

/// What makes two files identical: their bytes, or their decoded pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// The same file bytes (tier 0a).
    File,
    /// The same decoded pixels (tier 0b).
    Pixels,
}

/// The number of groups of identical files, and of the files in them.
pub fn identical_groups(conn: &Connection, by: Identity) -> Result<(i64, i64)> {
    let sql = match by {
        Identity::File => groups!("file_hash"),
        Identity::Pixels => groups!("pixel_hash"),
    };
    Ok(conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// The edges that join the files of each group of identical ones: each file to the one before it, in hash order.
pub fn identical_edges(conn: &Connection, by: Identity) -> Result<Vec<(FileId, FileId)>> {
    let sql = match by {
        Identity::File => grouped!("file_hash"),
        Identity::Pixels => grouped!("pixel_hash"),
    };
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut edges = Vec::new();
    // The previous row's hash and file.
    let mut prev_hash: Vec<u8> = Vec::new();
    let mut prev_id: Option<FileId> = None;
    while let Some(r) = rows.next()? {
        let (hash, id) = (r.get_ref(0)?.as_blob()?, FileId::new(r.get(1)?));
        if let Some(p) = prev_id
            && prev_hash == hash
        {
            edges.push((p, id));
        }
        prev_hash.clear();
        prev_hash.extend_from_slice(hash);
        prev_id = Some(id);
    }
    Ok(edges)
}

/// The number of present PDQ pairs whose distance is in `lo..=hi`.
pub fn pdq_pairs_between(conn: &Connection, lo: i64, hi: i64) -> Result<i64> {
    Ok(conn
        .query_row("SELECT count(*) FROM present_pdq_pairs WHERE distance BETWEEN ?1 AND ?2", [lo, hi], |r| r.get(0))?)
}

/// The present PDQ pairs at most `max` apart: the edges that group similar files.
pub fn pdq_edges(conn: &Connection, max: u32) -> Result<Vec<(FileId, FileId)>> {
    let mut stmt = conn.prepare("SELECT a, b FROM present_pdq_pairs WHERE distance <= ?1")?;
    let edges = stmt
        .query_map([max], |r| Ok((FileId::new(r.get(0)?), FileId::new(r.get(1)?))))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(edges)
}

/// Where a file is: its folder and name, as Windows spells it.
pub fn path(conn: &Connection, file_id: FileId) -> Result<String> {
    Ok(conn.query_row(
        "SELECT fo.path || '\\' || f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
        [file_id.get()],
        |r| r.get(0),
    )?)
}
