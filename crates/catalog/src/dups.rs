//! The duplicates' reads (tiers 0–1): identical files (`file_hash`), identical pixels (`pixel_hash`), and the PDQ pairs.
//! The counts for a report, and the edges that group files: the grouping itself (union-find) is `pg-dedup`'s.

use crate::Catalog;
use crate::error::Result;
use pg_model::ids::FileId;

/// The present files' current analyses.
macro_rules! current {
    () => {
        "FROM analysis a JOIN files f ON f.id = a.file_id WHERE a.error IS NULL AND f.missing = 0"
    };
}

/// The groups of a hash column (`file_hash`, tier 0a; `pixel_hash`, 0b): (how many, how many files in them). A file
/// without the hash (a video has no pixel hash) is in no group: GROUP BY would put all of them in one.
macro_rules! groups {
    ($column:literal) => {
        concat!(
            "SELECT count(*), coalesce(sum(n), 0) FROM
             (SELECT count(*) AS n ",
            current!(),
            " AND a.",
            $column,
            " IS NOT NULL GROUP BY a.",
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
            " AND a.",
            $column,
            " IS NOT NULL GROUP BY a.",
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
pub fn identical_groups(catalog: &Catalog, by: Identity) -> Result<(i64, i64)> {
    let conn = &catalog.conn;
    let sql = match by {
        Identity::File => groups!("file_hash"),
        Identity::Pixels => groups!("pixel_hash"),
    };
    Ok(conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// The edges that join the files of each group of identical ones: each file to the one before it, in hash order.
pub fn identical_edges(catalog: &Catalog, by: Identity) -> Result<Vec<(FileId, FileId)>> {
    let conn = &catalog.conn;
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

/// The number of present PDQ pairs whose distance is in each of `bands` (inclusive), in one pass over the pairs: the
/// report counts every band, and a scan per band read the pairs once each.
pub fn pdq_pairs_in_bands(catalog: &Catalog, bands: &[(u32, u32)]) -> Result<Vec<i64>> {
    if bands.is_empty() {
        return Ok(Vec::new());
    }
    let counts: Vec<String> = (0..bands.len())
        .map(|i| format!("coalesce(sum(distance BETWEEN ?{} AND ?{}), 0)", 2 * i + 1, 2 * i + 2))
        .collect();
    let sql = format!("SELECT {} FROM present_pdq_pairs", counts.join(", "));
    let values: Vec<i64> = bands.iter().flat_map(|&(lo, hi)| [i64::from(lo), i64::from(hi)]).collect();
    let conn = &catalog.conn;
    Ok(conn.query_row(&sql, rusqlite::params_from_iter(values), |r| {
        (0..bands.len()).map(|i| r.get(i)).collect::<rusqlite::Result<Vec<i64>>>()
    })?)
}

/// The present PDQ pairs at most `max` apart: the edges that group similar files.
pub fn pdq_edges(catalog: &Catalog, max: u32) -> Result<Vec<(FileId, FileId)>> {
    let conn = &catalog.conn;
    let mut stmt = conn.prepare("SELECT a, b FROM present_pdq_pairs WHERE distance <= ?1")?;
    let edges = stmt
        .query_map([max], |r| Ok((FileId::new(r.get(0)?), FileId::new(r.get(1)?))))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::mixed_library;
    use crate::fixture::raw;
    use pg_model::rules::PathRules;

    /// The edges as sorted (low, high) pairs: the order within a group is not part of the edges' contract.
    fn pairs(edges: Vec<(FileId, FileId)>) -> Vec<(i64, i64)> {
        let mut out: Vec<(i64, i64)> = edges
            .into_iter()
            .map(|(a, b)| {
                let (a, b) = (a.get(), b.get());
                (a.min(b), a.max(b))
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_file_without_the_hash_is_in_no_group() {
        let c = mixed_library(PathRules::Windows);
        // Files 1 and 3 share their bytes. No pixel hash is stored, so no file is in a pixel group (a video has none,
        // and GROUP BY would put every NULL in one group).
        assert_eq!(identical_groups(&c, Identity::File).unwrap(), (1, 2));
        assert_eq!(identical_groups(&c, Identity::Pixels).unwrap(), (0, 0));
        assert!(identical_edges(&c, Identity::Pixels).unwrap().is_empty());
        // Two images with the same pixels are a group; the video, with no hash, is not in it.
        raw::set_pixel_hash(&c, &[2, 4], &[9; 16]);
        assert_eq!(identical_groups(&c, Identity::Pixels).unwrap(), (1, 2));
        assert_eq!(pairs(identical_edges(&c, Identity::Pixels).unwrap()), [(2, 4)]);
    }

    #[test]
    fn the_pdq_bands_count_the_pairs_each_band_holds() {
        let c = mixed_library(PathRules::Windows);
        // The pairs are 2–4 at distance 10 and 2–6 at 30.
        let bands = [(0, 10), (11, 30), (0, 100), (31, 40)];
        let counts = pdq_pairs_in_bands(&c, &bands).unwrap();
        assert_eq!(counts, [1, 1, 2, 0]);
        // One pass over the pairs gives what a statement per band gives.
        for (&(lo, hi), n) in bands.iter().zip(&counts) {
            let one: i64 = c
                .conn
                .query_row(
                    "SELECT count(*) FROM present_pdq_pairs WHERE distance BETWEEN ?1 AND ?2",
                    [i64::from(lo), i64::from(hi)],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(one, *n);
        }
        assert!(pdq_pairs_in_bands(&c, &[]).unwrap().is_empty());
    }
}
