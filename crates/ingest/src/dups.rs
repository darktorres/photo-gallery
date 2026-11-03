//! Report of what tiers 0–1 found: identical files, identical pixels, and PDQ pairs, plus the groups they form
//! together. Grouping here is plain connected components, enough to inspect tiers 0–1. The real grouping is
//! multicut over verified tier-2 edges (PLAN §5, M3).

use std::collections::BTreeMap;

use anyhow::Result;
use pg_core::catalog::Catalog;

pub const DEFAULT_GROUP_DISTANCE: u32 = pg_model::pdq::GROUP_DISTANCE;

/// Groups of files by union. `parent` links each file to its root; `size` counts the members of each root's group,
/// and the smaller group joins the larger, so a chain is at most log₂ n long and `find`'s recursion stays shallow.
#[derive(Default)]
struct UnionFind {
    parent: BTreeMap<i64, i64>,
    size: BTreeMap<i64, i64>,
}

impl UnionFind {
    fn find(&mut self, x: i64) -> i64 {
        let p = *self.parent.entry(x).or_insert(x);
        if p == x {
            return x;
        }
        let root = self.find(p);
        self.parent.insert(x, root);
        root
    }

    fn union(&mut self, a: i64, b: i64) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let size = |s: &BTreeMap<i64, i64>, r: i64| s.get(&r).copied().unwrap_or(1);
        let (sa, sb) = (size(&self.size, ra), size(&self.size, rb));
        let (big, small) = if sa >= sb { (ra, rb) } else { (rb, ra) };
        self.parent.insert(small, big);
        self.size.insert(big, sa + sb);
    }
}

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

pub fn report(catalog: &Catalog, group_distance: u32, show: usize) -> Result<()> {
    let conn = &catalog.conn;
    for (tier, sql) in [("0a identical files", groups!("file_hash")), ("0b identical pixels", groups!("pixel_hash"))] {
        let (groups, files): (i64, i64) = conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        println!("tier {tier}: {groups} groups, {files} files");
    }
    println!("tier 1 PDQ pairs by distance:");
    for (lo, hi) in [(0, 0), (1, 8), (9, 22), (23, 31)] {
        let n: i64 =
            conn.query_row("SELECT count(*) FROM present_pdq_pairs WHERE distance BETWEEN ?1 AND ?2", [lo, hi], |r| {
                r.get(0)
            })?;
        println!("  {lo:>2}–{hi:<2}: {n}");
    }

    // Groups: tiers 0a and 0b, plus PDQ pairs up to `group_distance`.
    let mut uf = UnionFind::default();
    // The previous row's hash and file: each file is joined to the one before it with the same hash.
    let mut prev = Vec::new();
    for sql in [grouped!("file_hash"), grouped!("pixel_hash")] {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query([])?;
        let mut prev_id = None;
        while let Some(r) = rows.next()? {
            let (h, id) = (r.get_ref(0)?.as_blob()?, r.get(1)?);
            if let Some(p) = prev_id
                && prev == h
            {
                uf.union(p, id);
            }
            prev.clear();
            prev.extend_from_slice(h);
            prev_id = Some(id);
        }
    }
    let mut stmt = conn.prepare("SELECT a, b FROM present_pdq_pairs WHERE distance <= ?1")?;
    for pair in stmt.query_map([group_distance], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (a, b) = pair?;
        uf.union(a, b);
    }
    let ids: Vec<i64> = uf.parent.keys().copied().collect();
    // Members in id order; groups by size, then by their smallest id: the first member of each group seen.
    let mut smallest: BTreeMap<i64, i64> = BTreeMap::new();
    let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for id in ids {
        let root = uf.find(id);
        let first = *smallest.entry(root).or_insert(id);
        groups.entry(first).or_default().push(id);
    }
    let mut groups: Vec<Vec<i64>> = groups.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let files: usize = groups.iter().map(Vec::len).sum();
    println!(
        "groups (tiers 0a + 0b + PDQ ≤ {group_distance}): {} groups, {files} files, largest {:?}",
        groups.len(),
        groups.iter().take(5).map(Vec::len).collect::<Vec<_>>()
    );

    let mut path = conn.prepare(
        "SELECT fo.path || '\\' || f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.id = ?1",
    )?;
    for g in groups.iter().take(show) {
        println!("\n{} files:", g.len());
        for id in g.iter().take(20) {
            println!("  {}", path.query_row([id], |r| r.get::<_, String>(0))?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_chain_of_unions_stays_shallow() {
        // Each union joins a file to the one below it, from the top down, so the chain grows by one link each time. The
        // old union (the larger id under the smaller) made that a chain 200,000 deep, and `find` recursed once per
        // link. Here the smaller group joins the larger, so the depth stays about log₂ n.
        let mut uf = UnionFind::default();
        for i in (1..=200_000).rev() {
            uf.union(i - 1, i);
        }
        let root = uf.find(200_000);
        assert_eq!(uf.find(0), root);
    }
}
