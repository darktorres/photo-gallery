//! Pairing, at the end of every scan of a root (S-17.7 decisions 4, 7 and 8). The scan writes each file it finds by its
//! identity ([`ScanWriter::upsert_file`]); a file the catalog has no row for gets a fresh row, `unpaired`, and a row
//! whose identity is another file's is displaced, `unpaired`. This module decides what those rows are, in tiers:
//!
//! 1. **Same folder and name** on another volume: the file came back under a new identity (a renumbered volume), so
//!    its old row takes the new identity and keeps everything else.
//! 2. **Moved while closed**: the same file ID, size, mtime and extension on another volume whose files are all gone
//!    from this scan. Only on a complete scan, since the old volume's files must have been looked for.
//! 3. **Replacement** (a save over the same name): a fresh row with no children takes the row of the file it replaced,
//!    which loses its content state and keeps its ratings (decision 7). Only on a complete scan.
//!
//! A candidate counts when it matches exactly one row, every row counted (missing or not); several bind nothing and
//! are counted as ambiguous. A fresh row with children is never paired: pairing deletes it, and its children with it.
//! Each binding is in the one transaction of the pass.

use super::*;

/// What the pairing did, for the scan's report.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    /// Old rows that took the identity of a new row on another volume (tiers 1 and 2).
    pub rebound: usize,
    /// Rows that took the identity of the file that replaced them (decision 7).
    pub replaced: usize,
    /// Fresh rows still unpaired at the end whose candidates, in some tier, were several: nothing was bound to them.
    pub ambiguous: usize,
    /// Unpaired rows with children, which no tier pairs; counted on every scan.
    pub unpaired_with_data: usize,
    /// The rows whose content state was reset: their thumbnails are deleted by the caller, after the commit.
    pub replaced_ids: Vec<FileId>,
    /// The temp files of queued writes that were dropped with a replacement: deleted by the caller, after the commit.
    pub temps: Vec<String>,
}

/// A fresh row, as the pairing reads it.
struct Fresh {
    id: i64,
    volume: i64,
    file_ref: i64,
    created: Option<i64>,
    folder_id: i64,
    name: Vec<u8>,
    ext: String,
    kind: String,
    size: i64,
    mtime: i64,
    order_key: Vec<u8>,
}

impl Catalog {
    /// The pairing at the end of a scan of `root` (`scan` is its id). `clean` says the walk finished with every folder
    /// listed and none skipped: only then do tier 2, the replacements and the clearing run, since they need the whole
    /// tree to have been seen. Tier 1 runs on any scan that reached this point.
    pub fn pair(&mut self, root: &RulePath, scan: i64, clean: bool) -> Result<Pairing> {
        let under = under_params(self.rules, root);
        let tx = write_transaction(&self.conn)?;
        let mut out = Pairing::default();
        let fresh = fresh_rows(&tx, &under)?;
        // The fresh rows some tier found several candidates for; counted at the end, once each.
        let mut ambiguous = std::collections::BTreeSet::new();

        // Tier 1: the same folder and name, on another volume.
        {
            let mut same_name = tx.prepare(
                "SELECT id FROM files WHERE volume IS NOT NULL AND volume <> ?1 AND file_ref = ?2 AND folder_id = ?3
                 AND name = ?4 AND size = ?5 AND mtime = ?6 AND (created IS NULL OR ?7 IS NULL OR created = ?7)
                 AND last_scan < ?8 AND id <> ?9 ORDER BY id",
            )?;
            for f in &fresh {
                let ids: Vec<i64> = same_name
                    .query_map(
                        params![f.volume, f.file_ref, f.folder_id, f.name, f.size, f.mtime, f.created, scan, f.id],
                        |r| r.get(0),
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                match ids.as_slice() {
                    [] => {}
                    [only] => {
                        rebind(&tx, *only, f, scan)?;
                        out.rebound += 1;
                    }
                    _ => {
                        ambiguous.insert(f.id);
                    }
                }
            }
        }

        if clean {
            // Tier 2: the same file ID, size, mtime and extension, on a volume none of whose files this scan saw. The
            // volumes are read once here, not per candidate (a volume's rows, per candidate, was a quadratic scan).
            let mut seen = if fresh.is_empty() { Default::default() } else { seen_volumes(&tx, scan)? };
            let mut moved = tx.prepare(concat!(
                "SELECT id, volume FROM files WHERE volume IS NOT NULL AND volume <> ?4 AND file_ref = ?5 AND size = ?6
                 AND mtime = ?7 AND ext = ?8 AND (created IS NULL OR ?9 IS NULL OR created = ?9)
                 AND last_scan < ?10 AND id <> ?11 AND folder_id IN (",
                under_root!(),
                ") ORDER BY id"
            ))?;
            for f in fresh.iter().filter(|f| f.unpaired_still(&tx)) {
                let candidates: Vec<(i64, i64)> = moved
                    .query_map(
                        params![
                            under[0], under[1], under[2], f.volume, f.file_ref, f.size, f.mtime, f.ext, f.created,
                            scan, f.id
                        ],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                let ids: Vec<i64> =
                    candidates.into_iter().filter(|(_, volume)| !seen.contains(volume)).map(|(id, _)| id).collect();
                match ids.as_slice() {
                    [] => {}
                    [only] => {
                        rebind(&tx, *only, f, scan)?;
                        // The rebound row is on this scan's volume now: seen, as a row this scan saw would be.
                        seen.insert(f.volume);
                        out.rebound += 1;
                    }
                    _ => {
                        ambiguous.insert(f.id);
                    }
                }
            }

            // Replacement: the file that took a name, in the same folder, with a changed or unknown creation time.
            // Candidates are rows displaced or still unpaired, or live rows this scan did not see.
            let mut replaced_by = tx.prepare(
                "SELECT id FROM files WHERE folder_id = ?1 AND name = ?2 AND id <> ?3
                 AND ((unpaired = 1 AND (volume IS NULL OR last_scan < ?4))
                      OR (missing = 0 AND volume IS NOT NULL AND last_scan < ?4))
                 AND (created IS NULL OR ?5 IS NULL OR created <> ?5)
                 ORDER BY id",
            )?;
            for f in fresh.iter().filter(|f| f.unpaired_still(&tx)) {
                let ids: Vec<i64> = replaced_by
                    .query_map(params![f.folder_id, f.name, f.id, scan, f.created], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                match ids.as_slice() {
                    [] => {}
                    [only] => {
                        replace(&tx, *only, f, scan, &mut out.temps)?;
                        out.replaced += 1;
                        out.replaced_ids.push(FileId::new(*only));
                    }
                    _ => {
                        ambiguous.insert(f.id);
                    }
                }
            }
        }

        // Counted before the clearing below, which makes the rows nothing paired no longer unpaired.
        out.ambiguous = fresh.iter().filter(|f| ambiguous.contains(&f.id) && f.unpaired_still(&tx)).count();
        if clean {
            // The fresh rows nothing paired, which have no children, are genuinely new files: cleared. Displaced rows
            // (no identity) stay as they are, as tombstones a later file can replace.
            tx.execute(
                concat!(
                    "UPDATE files SET unpaired = 0 WHERE unpaired = 1 AND volume IS NOT NULL AND folder_id IN (",
                    under_root!(),
                    ") AND NOT ",
                    has_children!()
                ),
                params![under[0], under[1], under[2]],
            )?;
        }

        out.unpaired_with_data = tx.query_row(
            concat!(
                "SELECT count(*) FROM files WHERE unpaired = 1 AND folder_id IN (",
                under_root!(),
                ") AND ",
                has_children!()
            ),
            params![under[0], under[1], under[2]],
            |r| r.get::<_, usize>(0),
        )?;
        tx.commit()?;
        Ok(out)
    }
}

impl Fresh {
    /// Whether the row is still unpaired, after the passes so far: a bound row is gone or no longer unpaired.
    fn unpaired_still(&self, tx: &Transaction) -> bool {
        let unpaired = tx
            .prepare_cached("SELECT unpaired FROM files WHERE id = ?1")
            .and_then(|mut stmt| stmt.query_row([self.id], |r| r.get::<_, i64>(0)).optional());
        matches!(unpaired, Ok(Some(1)))
    }
}

/// The volumes of the files this scan saw (a row's `last_scan` is the scan's number).
fn seen_volumes(tx: &Transaction, scan: i64) -> Result<std::collections::BTreeSet<i64>> {
    let mut stmt = tx.prepare("SELECT DISTINCT volume FROM files WHERE last_scan = ?1 AND volume IS NOT NULL")?;
    Ok(stmt.query_map([scan], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

/// The fresh rows of the root that have an identity and no children: the rows to pair. A row with children is counted
/// by [`Catalog::pair`] and left alone.
fn fresh_rows(tx: &Transaction, under: &[Vec<u8>; 3]) -> Result<Vec<Fresh>> {
    let mut stmt = tx.prepare(concat!(
        "SELECT id, volume, file_ref, created, folder_id, name, ext, kind, size, mtime, order_key FROM files
         WHERE volume IS NOT NULL AND unpaired = 1 AND folder_id IN (",
        under_root!(),
        ") AND NOT ",
        has_children!(),
        " ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![under[0], under[1], under[2]], |r| {
            Ok(Fresh {
                id: r.get(0)?,
                volume: r.get(1)?,
                file_ref: r.get(2)?,
                created: r.get(3)?,
                folder_id: r.get(4)?,
                name: r.get(5)?,
                ext: r.get(6)?,
                kind: r.get(7)?,
                size: r.get(8)?,
                mtime: r.get(9)?,
                order_key: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Give the old row `id` the identity and the place of the fresh row `from`, which is deleted: the old row keeps its
/// id, ratings, rotation, analysis and content.
fn rebind(tx: &Transaction, id: i64, from: &Fresh, scan: i64) -> Result<()> {
    tx.execute("DELETE FROM files WHERE id = ?1", [from.id])?;
    tx.execute(
        "UPDATE files SET volume = ?2, file_ref = ?3, created = coalesce(?4, created), folder_id = ?5, name = ?6,
            ext = ?7, kind = ?8, size = ?9, mtime = ?10, last_scan = ?11, missing = 0, unpaired = 0, order_key = ?12
         WHERE id = ?1",
        params![
            id,
            from.volume,
            from.file_ref,
            from.created,
            from.folder_id,
            from.name,
            from.ext,
            from.kind,
            from.size,
            from.mtime,
            scan,
            from.order_key
        ],
    )?;
    Ok(())
}

/// Give the replaced row `id` the identity of the new file `from` (deleted), and reset what was made from the old file
/// (decision 7): the rotation and its queued turns, the analysis and what hangs off it, the file's own rating. The
/// queued ratings stay, without their temp files, which hold the old file's bytes; the rating row's `value` is written
/// again from the new file. The temp files to delete go to `temps`.
fn replace(tx: &Transaction, id: i64, from: &Fresh, scan: i64, temps: &mut Vec<String>) -> Result<()> {
    {
        let mut stmt = tx.prepare("SELECT temp FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL")?;
        for temp in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            temps.push(temp?);
        }
    }
    tx.execute("DELETE FROM file_writes WHERE file_id = ?1 AND kind = 'rotation'", [id])?;
    tx.execute("UPDATE file_writes SET temp = NULL, temp_value = NULL WHERE file_id = ?1 AND kind = 'rating'", [id])?;
    tx.execute("DELETE FROM file_ratings WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM analysis WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM border_sides WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM border_marks WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1", [id])?;
    tx.execute("DELETE FROM files WHERE id = ?1", [from.id])?;
    tx.execute(
        "UPDATE files SET volume = ?2, file_ref = ?3, created = ?4, folder_id = ?5, name = ?6, ext = ?7, kind = ?8,
            size = ?9, mtime = ?10, last_scan = ?11, missing = 0, unpaired = 0, rotation = 0, order_key = ?12
         WHERE id = ?1",
        params![
            id,
            from.volume,
            from.file_ref,
            from.created,
            from.folder_id,
            from.name,
            from.ext,
            from.kind,
            from.size,
            from.mtime,
            scan,
            from.order_key
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use pg_model::rules::PathRules;

    fn p(rules: PathRules, text: &str) -> RulePath {
        rules.parse(&spelled(rules, text)).expect("a test path")
    }

    fn catalog(rules: PathRules) -> Catalog {
        Catalog::prepare(std::path::Path::new(":memory:"), Versions { import: 1, borders: 1 }, rules).unwrap()
    }

    fn begin(c: &Catalog) -> i64 {
        let rules = c.rules();
        c.begin_scan(&p(rules, r"D:\x")).unwrap()
    }

    /// The scanner finding a file in D:\x with this identity, name, size and creation time (mtime 100).
    fn found(
        c: &mut Catalog,
        scan: i64,
        volume: u64,
        file: u64,
        name: &str,
        size: u64,
        created: Option<i64>,
    ) -> Recorded {
        let rules = c.rules();
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
        let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
        let file = ScannedFile {
            identity: FileIdentity::new(volume, file),
            name: &Name::from(name),
            ext: Ext::of_name(name).unwrap(),
            size,
            mtime: 100,
            created,
        };
        let recorded = writer.upsert_file(&folder, &file).unwrap();
        drop(writer);
        tx.commit().unwrap();
        recorded
    }

    fn pair(c: &mut Catalog, scan: i64, clean: bool) -> Pairing {
        let rules = c.rules();
        c.pair(&p(rules, r"D:\x"), scan, clean).unwrap()
    }

    fn count(c: &Catalog) -> i64 {
        c.conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap()
    }

    /// The row of the file called `name`: its id, its volume, and whether it is unpaired.
    fn row(c: &Catalog, name: &str) -> (i64, Option<i64>, bool) {
        c.conn
            .query_row("SELECT id, volume, unpaired FROM files WHERE name = ?1", [name.as_bytes()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1))
            })
            .unwrap()
    }

    fn a_file_on_another_volume_under_the_same_file_id_keeps_its_row(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        assert_eq!(pair(&mut c, first, true).rebound, 0);
        let (id, _, _) = row(&c, "a.jpg");
        c.conn.execute("INSERT INTO file_ratings(file_id, value) VALUES (?1, 4)", [id]).unwrap();

        // The volume was renumbered: the same file, under its new identity, in the same place.
        let second = begin(&c);
        assert_eq!(found(&mut c, second, 2, 7, "a.jpg", 10, Some(5)), Recorded::Changed);
        let paired = pair(&mut c, second, true);
        assert_eq!((paired.rebound, paired.ambiguous, paired.replaced), (1, 0, 0));
        assert_eq!(count(&c), 1);
        assert_eq!(row(&c, "a.jpg"), (id, Some(2), false));
        let kept: i64 =
            c.conn.query_row("SELECT count(*) FROM file_ratings WHERE file_id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(kept, 1);
    }

    under_both_rules!(
        a_file_on_another_volume_under_the_same_file_id_keeps_its_row,
        a_file_on_another_volume_under_the_same_file_id_keeps_its_row_windows,
        a_file_on_another_volume_under_the_same_file_id_keeps_its_row_posix
    );

    fn several_candidates_bind_nothing_and_count_as_one_ambiguous_row(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        found(&mut c, first, 3, 7, "a.jpg", 10, Some(5));
        pair(&mut c, first, true);

        // Tiers 1 and 2 both see two candidates for the new row: it is one ambiguous row, and stays unpaired.
        let second = begin(&c);
        found(&mut c, second, 2, 7, "a.jpg", 10, Some(5));
        let paired = pair(&mut c, second, true);
        assert_eq!((paired.rebound, paired.ambiguous), (0, 1));
        assert_eq!(count(&c), 3);
    }

    under_both_rules!(
        several_candidates_bind_nothing_and_count_as_one_ambiguous_row,
        several_candidates_bind_nothing_and_count_as_one_ambiguous_row_windows,
        several_candidates_bind_nothing_and_count_as_one_ambiguous_row_posix
    );

    fn a_file_renamed_while_closed_is_paired_on_a_clean_scan_only(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, first, true);
        let (id, _, _) = row(&c, "a.jpg");

        // Moved to another volume under another name: an incomplete scan does not pair it.
        let second = begin(&c);
        found(&mut c, second, 2, 7, "b.jpg", 10, Some(5));
        assert_eq!(pair(&mut c, second, false).rebound, 0);
        assert!(row(&c, "b.jpg").2, "still unpaired");

        // A clean scan, which looked for the old volume's files and found none of them, pairs it.
        let third = begin(&c);
        assert_eq!(pair(&mut c, third, true).rebound, 1);
        assert_eq!(count(&c), 1);
        assert_eq!(row(&c, "b.jpg"), (id, Some(2), false));
    }

    under_both_rules!(
        a_file_renamed_while_closed_is_paired_on_a_clean_scan_only,
        a_file_renamed_while_closed_is_paired_on_a_clean_scan_only_windows,
        a_file_renamed_while_closed_is_paired_on_a_clean_scan_only_posix
    );

    fn a_file_saved_over_keeps_its_row_and_rating_but_loses_its_content(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, first, true);
        let (id, _, _) = row(&c, "a.jpg");
        // The file's own rating, a queued rating whose temp file holds the old bytes, and a queued rotation.
        c.conn.execute("INSERT INTO file_ratings(file_id, value) VALUES (?1, 4)", [id]).unwrap();
        c.conn
            .execute(
                &format!(
                    "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                     VALUES (?1, 'rating', 3, 0, '{}', 3)",
                    spelled(rules, r"D:\x\a.jpg.tmp")
                ),
                [id],
            )
            .unwrap();
        c.conn
            .execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rotation', 1, 0)", [id])
            .unwrap();

        // Saved over: the name is taken by a new file, with another file ID and creation time.
        let second = begin(&c);
        assert_eq!(found(&mut c, second, 1, 9, "a.jpg", 12, Some(6)), Recorded::Changed);
        let paired = pair(&mut c, second, true);
        assert_eq!(paired.replaced, 1);
        assert_eq!(paired.replaced_ids, vec![FileId::new(id)]);
        assert_eq!(paired.temps, vec![spelled(rules, r"D:\x\a.jpg.tmp")]);
        assert_eq!(count(&c), 1);

        let (size, created, file_ref): (i64, Option<i64>, i64) = c
            .conn
            .query_row("SELECT size, created, file_ref FROM files WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((size, created, file_ref), (12, Some(6), 9));
        let ratings: i64 =
            c.conn.query_row("SELECT count(*) FROM file_ratings WHERE file_id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(ratings, 0, "the file's own rating is the old file's");
        let rotations: i64 = c
            .conn
            .query_row("SELECT count(*) FROM file_writes WHERE file_id = ?1 AND kind = 'rotation'", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(rotations, 0);
        let rating: (i64, bool) = c
            .conn
            .query_row(
                "SELECT value, temp IS NULL FROM file_writes WHERE file_id = ?1 AND kind = 'rating'",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rating, (3, true), "the queued rating stays, without its temp file");
    }

    under_both_rules!(
        a_file_saved_over_keeps_its_row_and_rating_but_loses_its_content,
        a_file_saved_over_keeps_its_row_and_rating_but_loses_its_content_windows,
        a_file_saved_over_keeps_its_row_and_rating_but_loses_its_content_posix
    );

    fn a_displaced_row_with_data_is_counted_and_never_paired(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, first, true);
        let (id, _, _) = row(&c, "a.jpg");
        c.conn.execute("INSERT INTO file_ratings(file_id, value) VALUES (?1, 4)", [id]).unwrap();

        // The identity is now another file, with another name and creation time: the old row is displaced.
        let second = begin(&c);
        assert_eq!(found(&mut c, second, 1, 7, "b.jpg", 10, Some(6)), Recorded::Changed);
        let paired = pair(&mut c, second, true);
        assert_eq!((paired.rebound, paired.replaced, paired.unpaired_with_data), (0, 0, 1));
        assert_eq!(count(&c), 2);
        assert_eq!(row(&c, "a.jpg"), (id, None, true));
        assert_eq!(row(&c, "b.jpg").1, Some(1));
        assert!(!row(&c, "b.jpg").2);
    }

    under_both_rules!(
        a_displaced_row_with_data_is_counted_and_never_paired,
        a_displaced_row_with_data_is_counted_and_never_paired_windows,
        a_displaced_row_with_data_is_counted_and_never_paired_posix
    );

    fn a_new_file_is_cleared_only_by_a_clean_scan(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, first, false);
        assert!(row(&c, "a.jpg").2, "an incomplete scan leaves it unpaired");

        let second = begin(&c);
        found(&mut c, second, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, second, true);
        assert!(!row(&c, "a.jpg").2);
    }

    under_both_rules!(
        a_new_file_is_cleared_only_by_a_clean_scan,
        a_new_file_is_cleared_only_by_a_clean_scan_windows,
        a_new_file_is_cleared_only_by_a_clean_scan_posix
    );
}
