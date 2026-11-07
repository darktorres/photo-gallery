//! The duplicates' groups: a union-find over the files' ids, by size, so a chain of unions stays shallow.

use pg_model::ids::FileId;
use std::collections::BTreeMap;

/// Groups of files by union. `parent` links each file to its root; `size` counts the members of each root's group,
/// and the smaller group joins the larger, so a chain is at most log₂ n long and `find`'s recursion stays shallow.
#[derive(Default)]
pub struct UnionFind {
    parent: BTreeMap<FileId, FileId>,
    size: BTreeMap<FileId, i64>,
}

impl UnionFind {
    /// Every id the structure has seen (a member of some group, or alone).
    pub fn members(&self) -> impl Iterator<Item = FileId> + '_ {
        self.parent.keys().copied()
    }

    pub fn find(&mut self, x: FileId) -> FileId {
        let p = *self.parent.entry(x).or_insert(x);
        if p == x {
            return x;
        }
        let root = self.find(p);
        self.parent.insert(x, root);
        root
    }

    pub fn union(&mut self, a: FileId, b: FileId) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let size = |s: &BTreeMap<FileId, i64>, r: FileId| s.get(&r).copied().unwrap_or(1);
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
            uf.union(FileId::new(i - 1), FileId::new(i));
        }
        let root = uf.find(FileId::new(200_000));
        assert_eq!(uf.find(FileId::new(0)), root);
    }
}
