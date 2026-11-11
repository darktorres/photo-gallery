//! Pairing, at the end of every scan of a root (S-17.7 decisions 4, 7 and 8). The scan writes each file it finds by its
//! identity ([`crate::ScanWriter::upsert_file`]); a file the catalog has no row for gets a fresh row, `unpaired`, and a row
//! whose identity is another file's is displaced, `unpaired`. This module decides what those rows are, in tiers:
//!
//! 1. **Same folder and name** on another volume: the file came back under a new identity (a renumbered volume), so
//!    its old row takes the new identity and keeps everything else.
//! 2. **Moved while closed**: the same file ID, size, mtime and extension on another volume whose files are all gone
//!    from this scan. Only on a complete scan, since the old volume's files must have been looked for.
//! 3. **Replacement** (a save over the same name): a fresh row with no children takes the row of the file it replaced,
//!    which loses its content state and keeps its ratings (decision 7). Only on a complete scan.
//!
//! The folders the scan did not list (unreadable ones, and the reparse folders it does not follow, with what is below
//! them) are out of every tier: their rows were not seen, so a row there is neither a candidate nor cleared, and the
//! rows paired are only those this scan saw (`last_scan`), which are in folders it listed. The rest of the root is
//! paired in full on a complete scan.
//!
//! A candidate counts when it matches exactly one row, every row counted (missing or not); several bind nothing and
//! are counted as ambiguous. A fresh row with children is never paired: pairing deletes it, and its children with it.
//! Each binding is in the one transaction of the pass.

use pg_model::ids::{FileId, ScanId};
use pg_model::media::{Ext, Kind};
use pg_model::rules::{Path as RulePath, PathRules};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::core::Catalog;
use crate::error::Result;
use crate::core::write_transaction;
use crate::types::{folders_under, skipped_folders, under_params};

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
    pub temps: Vec<RulePath>,
}

/// What a scan of a root saw, for its pairing.
#[derive(Clone, Copy, Debug)]
pub struct Coverage<'a> {
    /// The walk finished: it was not stopped. Tier 2, the replacements and the clearing need it.
    pub complete: bool,
    /// The folders the walk did not list: those it couldn't read, and the reparse folders it does not follow. Their
    /// subtrees were not seen, so their rows are left out of every tier.
    pub skipped: &'a [RulePath],
}

/// A fresh row, as the pairing reads it.
struct Fresh {
    id: i64,
    volume: i64,
    file_ref: i64,
    created: Option<i64>,
    folder_id: i64,
    name: Vec<u8>,
    ext: Ext,
    kind: Kind,
    size: i64,
    mtime: i64,
    order_key: Vec<u8>,
}

impl Catalog {
    /// The pairing at the end of a scan of `root` (`scan` is its id), over what `coverage` says the scan saw. Tier 1
    /// runs on any scan that reached this point. Tier 2, the replacements and the clearing run when the walk was
    /// complete, since they need the tree to have been looked through: they leave out the folders it skipped, whose
    /// rows were not seen, and pair only the fresh rows this scan saw.
    pub fn pair_after_scan(&mut self, root: &RulePath, scan: ScanId, coverage: Coverage<'_>) -> Result<Pairing> {
        let scan = scan.get();
        let under = under_params(self.rules, root);
        let tx = write_transaction(&self.conn)?;
        let skipped = skipped_folders(&tx, self.rules, coverage.skipped)?;
        let mut out = Pairing::default();
        // A row this scan saw was upserted in a folder it listed; one in a skipped folder is left out all the same.
        let fresh: Vec<Fresh> =
            fresh_rows(&tx, &under, scan)?.into_iter().filter(|f| !skipped.contains(&f.folder_id)).collect();
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

        if coverage.complete {
            // Tier 2: the same file ID, size, mtime and extension, on a volume none of whose files this scan saw, in a
            // folder it did not skip. The volumes are read once here, not per candidate (a volume's rows, per
            // candidate, was a quadratic scan).
            let mut seen = if fresh.is_empty() { Default::default() } else { seen_volumes(&tx, scan)? };
            let mut moved = tx.prepare(concat!(
                "SELECT id, volume, folder_id FROM files WHERE volume IS NOT NULL AND volume <> ?4 AND file_ref = ?5
                 AND size = ?6 AND mtime = ?7 AND ext = ?8 AND (created IS NULL OR ?9 IS NULL OR created = ?9)
                 AND last_scan < ?10 AND id <> ?11 AND folder_id IN (",
                under_root!(),
                ") ORDER BY id"
            ))?;
            for f in &fresh {
                // Checked here, not before the loop: a rebind earlier in it may have taken the row.
                if !f.unpaired_still(&tx)? {
                    continue;
                }
                let candidates: Vec<(i64, i64, i64)> = moved
                    .query_map(
                        params![
                            under[0], under[1], under[2], f.volume, f.file_ref, f.size, f.mtime, f.ext.name(), f.created, scan,
                            f.id
                        ],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                let ids: Vec<i64> = candidates
                    .into_iter()
                    .filter(|(_, volume, folder)| !seen.contains(volume) && !skipped.contains(folder))
                    .map(|(id, _, _)| id)
                    .collect();
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
            // Candidates are rows displaced or still unpaired, or live rows this scan did not see. They are in the
            // fresh row's folder, which the scan listed.
            let mut replaced_by = tx.prepare(
                "SELECT id FROM files WHERE folder_id = ?1 AND name = ?2 AND id <> ?3
                 AND ((unpaired = 1 AND (volume IS NULL OR last_scan < ?4))
                      OR (missing = 0 AND volume IS NOT NULL AND last_scan < ?4))
                 AND (created IS NULL OR ?5 IS NULL OR created <> ?5)
                 ORDER BY id",
            )?;
            for f in &fresh {
                if !f.unpaired_still(&tx)? {
                    continue;
                }
                let ids: Vec<i64> = replaced_by
                    .query_map(params![f.folder_id, f.name, f.id, scan, f.created], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                match ids.as_slice() {
                    [] => {}
                    [only] => {
                        replace(&tx, self.rules, *only, f, scan, &mut out.temps)?;
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
        for f in &fresh {
            if ambiguous.contains(&f.id) && f.unpaired_still(&tx)? {
                out.ambiguous += 1;
            }
        }
        if coverage.complete {
            // The fresh rows nothing paired, which have no children, are genuinely new files: cleared, outside the
            // skipped folders. Only the rows this scan saw are cleared: an unpaired row it did not see is a file that is
            // gone for now, and stays unpaired, so the file coming back under its identity is paired with its row (and
            // its data) rather than taken for a new file. Displaced rows (no identity) stay as they are, as tombstones a
            // later file can replace.
            let mut clear = tx.prepare(concat!(
                "UPDATE files SET unpaired = 0 WHERE folder_id = ?1 AND last_scan = ?2 AND unpaired = 1 \
                 AND volume IS NOT NULL AND NOT ",
                has_children!()
            ))?;
            for folder in folders_under(&tx, self.rules, root)? {
                if !skipped.contains(&folder) {
                    clear.execute(params![folder, scan])?;
                }
            }
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
    /// Whether the row is still unpaired, after the passes so far: a bound row is gone (false) or no longer unpaired.
    /// A failed read is an error, which fails the pass, not a row skipped.
    fn unpaired_still(&self, tx: &Transaction) -> Result<bool> {
        let unpaired: Option<i64> = tx
            .prepare_cached("SELECT unpaired FROM files WHERE id = ?1")?
            .query_row([self.id], |r| r.get(0))
            .optional()?;
        Ok(unpaired == Some(1))
    }
}

/// The volumes of the files this scan saw (a row's `last_scan` is the scan's number).
fn seen_volumes(tx: &Transaction, scan: i64) -> Result<std::collections::BTreeSet<i64>> {
    let mut stmt = tx.prepare("SELECT DISTINCT volume FROM files WHERE last_scan = ?1 AND volume IS NOT NULL")?;
    Ok(stmt.query_map([scan], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

/// The fresh rows of the root that this scan saw (`last_scan`), with an identity and no children: the rows to pair. A
/// row with children is counted by [`Catalog::pair_after_scan`] and left alone. A fresh row the scan did not see may be in a
/// folder it did not list, so it is not paired until a scan sees it.
fn fresh_rows(tx: &Transaction, under: &[Vec<u8>; 3], scan: i64) -> Result<Vec<Fresh>> {
    let mut stmt = tx.prepare(concat!(
        "SELECT id, volume, file_ref, created, folder_id, name, ext, kind, size, mtime, order_key FROM files
         WHERE volume IS NOT NULL AND unpaired = 1 AND last_scan = ?4 AND folder_id IN (",
        under_root!(),
        ") AND NOT ",
        has_children!(),
        " ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![under[0], under[1], under[2], scan], |r| {
            Ok(Fresh {
                id: r.get(0)?,
                volume: r.get(1)?,
                file_ref: r.get(2)?,
                created: r.get(3)?,
                folder_id: r.get(4)?,
                name: r.get(5)?,
                ext: crate::types::ext_at(r, 6)?,
                kind: crate::types::kind_at(r, 7)?,
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
            from.ext.name(),
            from.kind.key(),
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
fn replace(
    tx: &Transaction,
    rules: PathRules,
    id: i64,
    from: &Fresh,
    scan: i64,
    temps: &mut Vec<RulePath>,
) -> Result<()> {
    {
        let mut stmt = tx.prepare("SELECT temp FROM file_writes WHERE file_id = ?1 AND temp IS NOT NULL")?;
        for bytes in stmt.query_map([id], |r| r.get::<_, Vec<u8>>(0))? {
            let bytes = bytes?;
            let temp = rules
                .parse_bytes(&bytes)
                .ok_or_else(|| crate::error::Error::Other(anyhow::anyhow!("a queued temp path that doesn't parse")))?;
            temps.push(temp);
        }
    }
    tx.execute(concat!("DELETE FROM file_writes WHERE file_id = ?1 AND kind = '", pg_model::write_rotation!(), "'"), [id])?;
    tx.execute(
        concat!(
            "UPDATE file_writes SET temp = NULL, temp_value = NULL WHERE file_id = ?1 AND kind = '",
            pg_model::write_rating!(),
            "'"
        ),
        [id],
    )?;
    tx.execute("DELETE FROM file_ratings WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM analysis WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM border_sides WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM border_marks WHERE file_id = ?1", [id])?;
    tx.execute("DELETE FROM pdq_pairs WHERE a = ?1 OR b = ?1", [id])?;
    // Another file's content now: what failed on the old one is not known to fail (the file's own trigger misses a
    // replacement with the same size, time and turn).
    tx.execute("DELETE FROM file_failures WHERE file_id = ?1", [id])?;
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
            from.ext.name(),
            from.kind.key(),
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
    use crate::{Recorded, ScannedFile, Versions};
    use pg_model::ids::FileIdentity;
    use pg_model::media::Ext;
    use pg_model::name::Name;
    use pg_model::rules::PathRules;

    fn p(rules: PathRules, text: &str) -> RulePath {
        rules.parse(&spelled(rules, text)).expect("a test path")
    }

    fn catalog(rules: PathRules) -> Catalog {
        Catalog::prepare(std::path::Path::new(":memory:"), Versions { import: 1, borders: 1, thumbs: 1 }, rules)
            .unwrap()
    }

    fn begin(c: &Catalog) -> ScanId {
        let rules = c.rules();
        c.begin_scan(&p(rules, r"D:\x")).unwrap()
    }

    /// The scanner finding a file in D:\x with this identity, name, size and creation time (mtime 100).
    fn found(
        c: &mut Catalog,
        scan: ScanId,
        volume: u64,
        file: u64,
        name: &str,
        size: u64,
        created: Option<i64>,
    ) -> Recorded {
        found_in(c, scan, r"D:\x", volume, file, name, size, created)
    }

    /// [`found`], in the folder `folder` of the root D:\x.
    #[expect(clippy::too_many_arguments, reason = "a test helper: the facts of one scanned file")]
    fn found_in(
        c: &mut Catalog,
        scan: ScanId,
        folder: &str,
        volume: u64,
        file: u64,
        name: &str,
        size: u64,
        created: Option<i64>,
    ) -> Recorded {
        let rules = c.rules();
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
        let folder = writer.folder(&p(rules, folder)).unwrap();
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

    /// The pairing of a scan of D:\x that skipped no folder, `complete` or stopped.
    fn pair(c: &mut Catalog, scan: i64, complete: bool) -> Pairing {
        let rules = c.rules();
        c.pair_after_scan(&p(rules, r"D:\x"), scan, Coverage { complete, skipped: &[] }).unwrap()
    }

    /// The pairing of a complete scan of D:\x that did not list `skipped` (the folders' texts).
    fn pair_skipping(c: &mut Catalog, scan: i64, skipped: &[&str]) -> Pairing {
        let rules = c.rules();
        let skipped: Vec<RulePath> = skipped.iter().map(|text| p(rules, text)).collect();
        c.pair_after_scan(&p(rules, r"D:\x"), scan, Coverage { complete: true, skipped: &skipped }).unwrap()
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

        // Tiers 1 and 2 both see two candidates for the new row: nothing is bound to it, and it counts as one ambiguous
        // row. It is then cleared as a new file (the scan is complete and it has no children), so it isn't unpaired.
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

        // A complete scan, which looked for the old volume's files and found none of them, pairs it.
        let third = begin(&c);
        assert_eq!(found(&mut c, third, 2, 7, "b.jpg", 10, Some(5)), Recorded::Same);
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
        let temp = rules.join(&p(rules, r"D:\x\a.jpg.tmp"));
        c.conn
            .execute(
                "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                 VALUES (?1, 'rating', 3, 0, ?2, 3)",
                rusqlite::params![id, temp],
            )
            .unwrap();
        c.conn
            .execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (?1, 'rotation', 1, 0)", [id])
            .unwrap();
        // Its thumbnail failed.
        c.conn
            .execute(
                "INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen)
                 SELECT id, 'thumbs', mtime, size, rotation, 'it panicked', 0, 0 FROM files WHERE id = ?1",
                [id],
            )
            .unwrap();

        // Saved over: the name is taken by a new file, with another file ID and creation time.
        let second = begin(&c);
        assert_eq!(found(&mut c, second, 1, 9, "a.jpg", 12, Some(6)), Recorded::Changed);
        let paired = pair(&mut c, second, true);
        assert_eq!(paired.replaced, 1);
        assert_eq!(paired.replaced_ids, vec![FileId::new(id)]);
        assert_eq!(paired.temps, vec![p(rules, r"D:\x\a.jpg.tmp")]);
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
        let failures: i64 =
            c.conn.query_row("SELECT count(*) FROM file_failures WHERE file_id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(failures, 0, "the old file's failure is not the new one's");
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

    /// A complete scan that skipped a folder (unreadable, or a reparse folder) pairs and clears the rest of the root,
    /// and leaves the skipped folder's rows as they are: a row there is no tier-2 candidate, and a fresh row there is
    /// neither paired nor cleared.
    fn a_skipped_folders_rows_are_not_paired_or_cleared(rules: PathRules) {
        let mut c = catalog(rules);
        // old.jpg in sub, a known file; new.jpg in sub, found by a scan that was stopped (still unpaired).
        let first = begin(&c);
        found_in(&mut c, first, r"D:\x\sub", 1, 7, "old.jpg", 10, Some(5));
        pair(&mut c, first, true);
        let (old, _, _) = row(&c, "old.jpg");
        let stopped = begin(&c);
        found_in(&mut c, stopped, r"D:\x\sub\deeper", 1, 8, "new.jpg", 10, Some(5));
        pair(&mut c, stopped, false);
        assert!(row(&c, "new.jpg").2);

        // Now sub can't be read. A file on another volume with old.jpg's file ID, size, mtime and creation time shows
        // up in the root: old.jpg's volume is not seen, but its folder was not looked in, so it is not a candidate.
        let third = begin(&c);
        found(&mut c, third, 2, 7, "moved.jpg", 10, Some(5));
        let paired = pair_skipping(&mut c, third, &[r"D:\x\sub"]);
        assert_eq!((paired.rebound, paired.replaced, paired.ambiguous), (0, 0, 0));
        assert_eq!(count(&c), 3);
        assert_eq!(row(&c, "old.jpg"), (old, Some(1), false));
        assert!(row(&c, "new.jpg").2, "a fresh row in a skipped folder is not cleared");
        assert!(!row(&c, "moved.jpg").2, "the rest of the root is cleared");

        // Read again: the fresh row the scan sees there is cleared.
        let fourth = begin(&c);
        found(&mut c, fourth, 2, 7, "moved.jpg", 10, Some(5));
        found_in(&mut c, fourth, r"D:\x\sub\deeper", 1, 8, "new.jpg", 10, Some(5));
        pair(&mut c, fourth, true);
        assert!(!row(&c, "new.jpg").2);
    }

    under_both_rules!(
        a_skipped_folders_rows_are_not_paired_or_cleared,
        a_skipped_folders_rows_are_not_paired_or_cleared_windows,
        a_skipped_folders_rows_are_not_paired_or_cleared_posix
    );

    /// A stopped scan leaves a new file unpaired in a folder it listed, and a later complete scan does not see that
    /// file. The later scan pairs only the fresh rows it saw (`fresh_rows`' `last_scan`), so the unseen row is no
    /// candidate: the live row on another volume with the same file ID and facts (a moved-while-closed candidate, on a
    /// volume the later scan did not see) is not rebound to it. Without that filter, tier 2 would rebind it.
    fn a_fresh_row_the_later_complete_scan_did_not_see_is_not_paired(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        // c.jpg: a live row on volume 2, with file ID 8.
        found(&mut c, first, 2, 8, "c.jpg", 10, Some(5));
        pair(&mut c, first, true);
        let (live, _, _) = row(&c, "c.jpg");

        // A stopped scan finds b.jpg: file ID 8 on volume 1, unpaired in the listed folder D:\x.
        let stopped = begin(&c);
        found(&mut c, stopped, 1, 8, "b.jpg", 10, Some(5));
        assert_eq!(pair(&mut c, stopped, false).rebound, 0);
        assert!(row(&c, "b.jpg").2, "still unpaired");

        // A later complete scan sees a.jpg and not b.jpg. Volume 2 is not among the volumes it saw.
        let later = begin(&c);
        assert_eq!(found(&mut c, later, 1, 7, "a.jpg", 10, Some(5)), Recorded::Same);
        let paired = pair(&mut c, later, true);
        assert_eq!((paired.rebound, paired.replaced, paired.ambiguous), (0, 0, 0));
        assert_eq!(row(&c, "c.jpg"), (live, Some(2), false), "the live row on the other volume is not rebound");
        assert_eq!(count(&c), 3);
    }

    /// A complete scan that doesn't see an unpaired live row leaves it unpaired: its file is gone for now, and when it
    /// comes back under its identity, the row (with its data) is paired with it, rather than a new row being made.
    fn an_unpaired_row_a_complete_scan_did_not_see_stays_unpaired_for_its_return(rules: PathRules) {
        let mut c = catalog(rules);
        let first = begin(&c);
        found(&mut c, first, 1, 7, "a.jpg", 10, Some(5));
        // b.jpg: a live row on volume 1 that a stopped scan finds, so it is unpaired.
        let stopped = begin(&c);
        found(&mut c, stopped, 1, 8, "b.jpg", 10, Some(5));
        assert_eq!(pair(&mut c, stopped, false).rebound, 0);
        assert!(row(&c, "b.jpg").2, "unpaired after the stopped scan");

        // A complete scan that doesn't see b.jpg: it is gone for now, so it stays unpaired.
        let gone = begin(&c);
        found(&mut c, gone, 1, 7, "a.jpg", 10, Some(5));
        pair(&mut c, gone, true);
        assert!(row(&c, "b.jpg").2, "still unpaired: a complete scan that didn't see it doesn't clear it");
        assert_eq!(count(&c), 2);

        // The file is back under its identity: it is paired with its row, not made a new one.
        let back = begin(&c);
        found(&mut c, back, 1, 7, "a.jpg", 10, Some(5));
        found(&mut c, back, 1, 8, "b.jpg", 10, Some(5));
        pair(&mut c, back, true);
        assert!(!row(&c, "b.jpg").2, "paired with its returning file");
        assert_eq!(count(&c), 2, "no new row for the returning file");
    }

    under_both_rules!(
        an_unpaired_row_a_complete_scan_did_not_see_stays_unpaired_for_its_return,
        an_unpaired_row_a_complete_scan_did_not_see_stays_unpaired_for_its_return_windows,
        an_unpaired_row_a_complete_scan_did_not_see_stays_unpaired_for_its_return_posix
    );

    under_both_rules!(
        a_fresh_row_the_later_complete_scan_did_not_see_is_not_paired,
        a_fresh_row_the_later_complete_scan_did_not_see_is_not_paired_windows,
        a_fresh_row_the_later_complete_scan_did_not_see_is_not_paired_posix
    );
}
