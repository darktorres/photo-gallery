//! The pairing pass: the tier-1 pairs of the pictures that owe them (`analysis.dups_state = 'due'`, set by the import),
//! against every picture with orientation hashes. The join (`pg_dedup::join`) finds them in slices of the owed pictures,
//! and each slice is written in short transactions: a pair with its guards (`PairsWriter`), then the picture marked
//! done. A stop or a crash keeps what was written; what is left stays owed, and the next pass makes it.
//!
//! Which pairs a picture owns: a pair (a, b) with `a < b` (by file id) is owned by `b` if `b` is owed, else by `a` (it is
//! owed: the pass never pairs two pictures that owe nothing). The owner's slice is the one that queries it, so each pair
//! is found and written once, and a picture is done only after every pair it owns is written.

use anyhow::Result;
use pg_catalog::Catalog;
use pg_dedup::join::{self, Pair, Picture};
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::num::index;
use pg_model::progress::Phase;
use pg_platform::priority;

/// The owed pictures per slice of the join: bounds a slice's pairs in memory, and makes the writes incremental.
const SLICE: usize = 50_000;
/// The owed pictures per write transaction.
const WRITE: usize = 2_000;

/// What a pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PairStats {
    /// The owed pictures paired (their pairs stored where they are found, and marked done).
    pub files: usize,
    /// The pairs stored.
    pub pairs: usize,
    /// The analyses left out for having no orientation hashes (`PairingWork::legacy`).
    pub legacy: usize,
    /// Whether every owed picture was paired (false: the pass was stopped).
    pub complete: bool,
}

/// Pairs what is owed. Nothing owed is one probe, and no hash is read.
pub fn pair(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<PairStats> {
    if catalog.owed_pairs()? == 0 {
        return Ok(PairStats { complete: true, ..PairStats::default() });
    }
    let work = catalog.pairing_work()?;
    let owed: Vec<usize> = work.pictures.iter().enumerate().filter(|(_, p)| p.due).map(|(i, _)| i).collect();
    let mut stats = PairStats { legacy: work.legacy, complete: true, ..PairStats::default() };
    if owed.is_empty() {
        return Ok(stats);
    }
    let pictures: Vec<Picture> =
        work.pictures.iter().map(|p| Picture { orientations: p.orientations, due: p.due }).collect();

    // The phase ends when this is dropped, however the pass leaves. Its progress is the owed pictures paired so far.
    let _phase = ctl.begin(Phase::Pairing, u64::try_from(owed.len()).unwrap_or(u64::MAX));
    let start = || priority::set_current_thread(opts.priority);
    let pause = || ctl.checkpoint();

    'slices: for slice in owed.chunks(SLICE) {
        let (Some(&first), Some(&last)) = (slice.first(), slice.last()) else { continue };
        let Some(mut pairs) = join::join(&pictures, first..last + 1, opts.workers, &start, &pause)? else {
            stats.complete = false;
            break;
        };
        pairs.sort_unstable_by_key(|p| owner(p, &pictures));
        let mut at = 0;
        for batch in slice.chunks(WRITE) {
            // A stop between transactions leaves them as they are; the rest of the slice stays owed.
            if !ctl.checkpoint() {
                stats.complete = false;
                break 'slices;
            }
            let tx = catalog.tx()?;
            let mut writer = tx.pairs_writer()?;
            for &i in batch {
                while let Some(pair) = pairs.get(at).filter(|p| owner(p, &pictures) == i) {
                    let (a, b) = (index(pair.a), index(pair.b));
                    let (pa, pb) = (work.pictures.get(a), work.pictures.get(b));
                    if let (Some(pa), Some(pb)) = (pa, pb)
                        && writer.pair(pa.id, pb.id, pair.distance, &pa.pdq.0, &pb.pdq.0)?
                    {
                        stats.pairs += 1;
                    }
                    at += 1;
                }
                // Every pair the picture owns is in its transaction: the picture is done, if it still holds the hash
                // they were made from.
                if let Some(pic) = work.pictures.get(i)
                    && writer.done(pic.id, &pic.pdq.0)?
                {
                    stats.files += 1;
                }
            }
            drop(writer);
            tx.commit()?;
            ctl.set_done(u64::try_from(stats.files).unwrap_or(u64::MAX));
        }
    }
    Ok(stats)
}

/// The picture that owns the pair (see the module doc).
fn owner(pair: &Pair, pictures: &[Picture]) -> usize {
    let b = index(pair.b);
    if pictures.get(b).is_some_and(|p| p.due) { b } else { index(pair.a) }
}
