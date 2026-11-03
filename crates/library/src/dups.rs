//! What tiers 0–1 found, as data: identical files, identical pixels, and PDQ pairs, plus the groups they form together.
//! Grouping here is plain connected components, enough to inspect tiers 0–1. The real grouping is multicut over
//! verified tier-2 edges (PLAN §5, M3). The report that prints it is the command line's (`pg_cli::dups`).

use std::collections::BTreeMap;

use anyhow::Result;
use pg_catalog::Catalog;
use pg_catalog::dups::{self, Identity};
use pg_dedup::UnionFind;
use pg_model::ids::FileId;

/// What tiers 0–1 found.
pub struct Found {
    /// Per identity, in the order of `IDENTITIES`: the groups of identical files and the files in them.
    pub identical: Vec<(Identity, i64, i64)>,
    /// The PDQ pairs in each distance band, as `(lo, hi, pairs)`.
    pub pdq_bands: Vec<(u32, u32, i64)>,
    /// The groups of two or more files that tiers 0a, 0b and PDQ up to `group_distance` form, largest first. Each
    /// lists its files in id order.
    pub groups: Vec<Vec<FileId>>,
}

/// The identities of tiers 0a (identical files) and 0b (identical pixels).
pub const IDENTITIES: [Identity; 2] = [Identity::File, Identity::Pixels];

/// The PDQ distance bands the report counts pairs in.
pub const PDQ_BANDS: [(u32, u32); 4] = [(0, 0), (1, 8), (9, 22), (23, 31)];

pub fn find(catalog: &Catalog, group_distance: u32) -> Result<Found> {
    let mut identical = Vec::with_capacity(IDENTITIES.len());
    for by in IDENTITIES {
        let (groups, files) = dups::identical_groups(catalog, by)?;
        identical.push((by, groups, files));
    }
    let mut pdq_bands = Vec::with_capacity(PDQ_BANDS.len());
    for (lo, hi) in PDQ_BANDS {
        pdq_bands.push((lo, hi, dups::pdq_pairs_between(catalog, lo.into(), hi.into())?));
    }

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
