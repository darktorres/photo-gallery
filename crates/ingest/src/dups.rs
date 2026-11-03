//! Report of what tiers 0–1 found: identical files, identical pixels, and PDQ pairs, plus the groups they form
//! together. Grouping here is plain connected components, enough to inspect tiers 0–1. The real grouping is
//! multicut over verified tier-2 edges (PLAN §5, M3).

use std::collections::BTreeMap;

use anyhow::Result;
use pg_catalog::Catalog;
use pg_catalog::dups::{self, Identity};
use pg_dedup::UnionFind;
use pg_model::ids::FileId;

pub const DEFAULT_GROUP_DISTANCE: u32 = pg_model::pdq::GROUP_DISTANCE;

pub fn report(catalog: &Catalog, group_distance: u32, show: usize) -> Result<()> {
    for (tier, by) in [("0a identical files", Identity::File), ("0b identical pixels", Identity::Pixels)] {
        let (groups, files) = dups::identical_groups(catalog, by)?;
        println!("tier {tier}: {groups} groups, {files} files");
    }
    println!("tier 1 PDQ pairs by distance:");
    for (lo, hi) in [(0, 0), (1, 8), (9, 22), (23, 31)] {
        println!("  {lo:>2}–{hi:<2}: {}", dups::pdq_pairs_between(catalog, lo, hi)?);
    }

    // Groups: tiers 0a and 0b, plus PDQ pairs up to `group_distance`.
    let mut uf = UnionFind::default();
    for by in [Identity::File, Identity::Pixels] {
        for (a, b) in dups::identical_edges(catalog, by)? {
            uf.union(a, b);
        }
    }
    for (a, b) in dups::pdq_edges(catalog, group_distance)? {
        uf.union(a, b);
    }
    let ids: Vec<FileId> = uf.members().collect();
    // Members in id order; groups by size, then by their smallest id: the first member of each group seen.
    let mut smallest: BTreeMap<FileId, FileId> = BTreeMap::new();
    let mut groups: BTreeMap<FileId, Vec<FileId>> = BTreeMap::new();
    for id in ids {
        let root = uf.find(id);
        let first = *smallest.entry(root).or_insert(id);
        groups.entry(first).or_default().push(id);
    }
    let mut groups: Vec<Vec<FileId>> = groups.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let files: usize = groups.iter().map(Vec::len).sum();
    println!(
        "groups (tiers 0a + 0b + PDQ ≤ {group_distance}): {} groups, {files} files, largest {:?}",
        groups.len(),
        groups.iter().take(5).map(Vec::len).collect::<Vec<_>>()
    );

    for g in groups.iter().take(show) {
        println!("\n{} files:", g.len());
        for id in g.iter().take(20) {
            println!("  {}", catalog.file_path(*id)?);
        }
    }
    Ok(())
}
