//! The `dups` report: what tiers 0–1 found, as data (identical files, identical pixels, and PDQ pairs, plus the groups
//! they form together), and printed. Grouping here is plain connected components, enough to inspect tiers 0–1. The real
//! grouping is multicut over verified tier-2 edges (PLAN §5, M3).

use std::collections::BTreeMap;

use anyhow::Result;
use pg_catalog::Catalog;
use pg_catalog::dups::{self, Identity};
use pg_dedup::UnionFind;
use pg_model::ids::FileId;

/// The PDQ distance that joins two files into a group by default.
pub const DEFAULT_GROUP_DISTANCE: u32 = pg_model::pdq::GROUP_DISTANCE;

/// The identities of tiers 0a (identical files) and 0b (identical pixels).
const IDENTITIES: [Identity; 2] = [Identity::File, Identity::Pixels];

/// The PDQ distance bands the report counts pairs in.
const PDQ_BANDS: [(u32, u32); 4] = [(0, 0), (1, 8), (9, 22), (23, 31)];

/// What tiers 0–1 found.
struct Found {
    /// Per identity, in the order of `IDENTITIES`: the groups of identical files and the files in them.
    identical: Vec<(Identity, i64, i64)>,
    /// The PDQ pairs in each distance band, as `(lo, hi, pairs)`.
    pdq_bands: Vec<(u32, u32, i64)>,
    /// The groups of two or more files that tiers 0a, 0b and PDQ up to `group_distance` form, largest first. Each
    /// lists its files in id order.
    groups: Vec<Vec<FileId>>,
}

/// Prints the report, with the files of the `show` largest groups.
pub fn report(catalog: &Catalog, group_distance: u32, show: usize) -> Result<()> {
    let found = find(catalog, group_distance)?;
    for (by, groups, files) in &found.identical {
        let tier = match by {
            Identity::File => "0a identical files",
            Identity::Pixels => "0b identical pixels",
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

/// What tiers 0–1 found in `catalog`, grouping by PDQ up to `group_distance`.
fn find(catalog: &Catalog, group_distance: u32) -> Result<Found> {
    let mut identical = Vec::with_capacity(IDENTITIES.len());
    for by in IDENTITIES {
        let (groups, files) = dups::identical_groups(catalog, by)?;
        identical.push((by, groups, files));
    }
    let counts = dups::pdq_pairs_in_bands(catalog, &PDQ_BANDS)?;
    let pdq_bands = PDQ_BANDS.iter().zip(counts).map(|(&(lo, hi), pairs)| (lo, hi, pairs)).collect();

    let mut uf = UnionFind::default();
    for by in IDENTITIES {
        for (a, b) in dups::identical_edges(catalog, by)? {
            uf.union(a, b);
        }
    }
    for (a, b) in dups::pdq_edges(catalog, group_distance)? {
        uf.union(a, b);
    }
    // Members in id order; groups by size, then by their smallest id: the first member of each group seen.
    let mut smallest: BTreeMap<FileId, FileId> = BTreeMap::new();
    let mut by_first: BTreeMap<FileId, Vec<FileId>> = BTreeMap::new();
    let ids: Vec<FileId> = uf.members().collect();
    for id in ids {
        let root = uf.find(id);
        let first = *smallest.entry(root).or_insert(id);
        by_first.entry(first).or_default().push(id);
    }
    let mut groups: Vec<Vec<FileId>> = by_first.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    Ok(Found { identical, pdq_bands, groups })
}
