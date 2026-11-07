//! The `dups` report: what tiers 0–1 found (`pg_library::dups`), printed.

use anyhow::Result;
use pg_catalog::Catalog;
use pg_library::dups::{self, Found};

/// The PDQ distance that joins two files into a group by default.
pub const DEFAULT_GROUP_DISTANCE: u32 = pg_model::pdq::GROUP_DISTANCE;

/// Prints the report, with the files of the `show` largest groups.
pub fn report(catalog: &Catalog, group_distance: u32, show: usize) -> Result<()> {
    let found: Found = dups::find(catalog, group_distance)?;
    for (by, groups, files) in &found.identical {
        let tier = match by {
            pg_catalog::dups::Identity::File => "0a identical files",
            pg_catalog::dups::Identity::Pixels => "0b identical pixels",
        };
        println!("tier {tier}: {groups} groups, {files} files");
    }
    println!("tier 1 PDQ pairs by distance:");
    for (lo, hi, pairs) in &found.pdq_bands {
        println!("  {lo:>2}–{hi:<2}: {pairs}");
    }
    let files: usize = found.groups.iter().map(Vec::len).sum();
    println!(
        "groups (tiers 0a + 0b + PDQ ≤ {group_distance}): {} groups, {files} files, largest {:?}",
        found.groups.len(),
        found.groups.iter().take(5).map(Vec::len).collect::<Vec<_>>()
    );
    for g in found.groups.iter().take(show) {
        println!("\n{} files:", g.len());
        for id in g.iter().take(20) {
            let path =
                catalog.file_path(*id)?.ok_or_else(|| anyhow::anyhow!("file {} is not in the catalog", id.get()))?;
            println!("  {}", catalog.rules().display(&path));
        }
    }
    Ok(())
}
