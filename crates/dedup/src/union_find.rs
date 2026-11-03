//! The duplicates' groups: a union-find over the files' ids, by size, so a chain of unions stays shallow.

use std::collections::BTreeMap;

/// Groups of files by union. `parent` links each file to its root; `size` counts the members of each root's group,
/// and the smaller group joins the larger, so a chain is at most log₂ n long and `find`'s recursion stays shallow.
#[derive(Default)]
pub struct UnionFind {
    parent: BTreeMap<i64, i64>,
    size: BTreeMap<i64, i64>,
}

impl UnionFind {
    /// Every id the structure has seen (a member of some group, or alone).
    pub fn members(&self) -> impl Iterator<Item = i64> + '_ {
        self.parent.keys().copied()
    }

    pub fn find(&mut self, x: i64) -> i64 {
        let p = *self.parent.entry(x).or_insert(x);
        if p == x {
            return x;
        }
        let root = self.find(p);
        self.parent.insert(x, root);
        root
    }

    pub fn union(&mut self, a: i64, b: i64) {
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
