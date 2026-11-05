//! Borders (pg_analysis::borders): the pass after the import that looks for them in the files it analyzed, and in
//! every file after a new detector (`analysis.borders_due`), each decoded at full size. The tools that report on
//! borders, or take review marks into the catalog, are the command line's (`pg_cli::borders`).

use anyhow::Result;
use pg_analysis::analyze::{Analyzer, FileBorders};
use pg_catalog::Catalog;
use pg_jobs::control::{Control, Phase};
use pg_jobs::pipeline::{self, Options};

const BATCH: usize = 500;

pub struct Stats {
    pub files: usize,
    pub failed: usize,
}

/// Look for the borders that are due, committing in batches (stopping loses at most the batch).
pub fn find(catalog: &mut Catalog, opts: &Options, ctl: &Control) -> Result<Stats> {
    let work = catalog.pending_borders()?;
    let items = &work.items;
    let mut stats = Stats { files: 0, failed: 0 };
    if items.is_empty() {
        return Ok(stats);
    }
    ctl.begin(Phase::Borders, items.len() as u64);
    let mut batch: Vec<(usize, Result<FileBorders>)> = Vec::with_capacity(BATCH);
    let mut flush = |batch: &mut Vec<(usize, Result<FileBorders>)>, stats: &mut Stats| -> Result<()> {
        let tx = catalog.tx()?;
        let mut writer = tx.borders_writer()?;
        for (i, result) in batch.drain(..) {
            stats.files += 1;
            let found = match result {
                Ok(b) => b.borders,
                // Couldn't be read (a locked file, an offline drive): nothing is stored, so the file stays due and the
                // next pass tries it again. Storing "no borders" here would hide it until BORDERS_VERSION changed (R-47).
                Err(e) if e.downcast_ref::<std::io::Error>().is_some() => {
                    stats.failed += 1;
                    continue;
                }
                // It doesn't decode: no borders, for the file as it is; the import records the failure.
                Err(_) => {
                    stats.failed += 1;
                    None
                }
            };
            writer.store_borders(&items[i], found.as_ref())?;
        }
        drop(writer);
        tx.commit()?;
        Ok(())
    };
    pipeline::run(
        items,
        |item, wide| {
            let [folder, name] = work.parts(item.path);
            pg_platform::names::set_file(wide, folder, name);
            true
        },
        opts,
        ctl,
        Analyzer::new,
        |analyzer, i, bytes| analyzer.borders(bytes, items[i].rotation),
        |i, result, len| {
            ctl.add(len, result.is_err());
            batch.push((i, result));
            if batch.len() >= BATCH {
                flush(&mut batch, &mut stats)?;
            }
            Ok(true)
        },
    )?;
    flush(&mut batch, &mut stats)?;
    ctl.finish();
    Ok(stats)
}
