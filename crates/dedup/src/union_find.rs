//! The duplicates' groups: a union-find over the files the edges name. Each file is numbered by its position in the
//! sorted list of their ids, found once per edge endpoint when the structure is made; the links after that are `u32`s.

use pg_model::ids::FileId;
use pg_model::num::index;

/// The edges name more files than a `u32` can number. A catalog holds fewer than 2³² files, so this is not reached by
/// a catalog; it is the error rather than a panic in case it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the duplicates name more files than a 32-bit number can count")]
pub struct TooManyFiles;

/// Groups of files by union. The files are numbered by position in `ids` (sorted, distinct): `parent` links each
/// number to its root's, and `size` counts the members of each root's group. The smaller group joins the larger, so a
/// chain is at most log₂ n links long.
pub struct UnionFind {
    ids: Vec<FileId>,
    /// The number of files, which is `ids.len()` as a `u32` (checked when the structure is made).
    count: u32,
    parent: Vec<u32>,
    size: Vec<u32>,
}

#[expect(
    clippy::indexing_slicing,
    reason = "a number is a position in `ids`, or a root's number in it: the three vectors have one length"
)]
impl UnionFind {
    /// The groups the edges make: the files of each edge, and the files linked to them through the edges, in one group.
    /// Every endpoint is found in the sorted ids once, here.
    pub fn from_edges(edges: &[(FileId, FileId)]) -> Result<UnionFind, TooManyFiles> {
        let mut ids: Vec<FileId> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
        ids.sort_unstable();
        ids.dedup();
        let count = u32::try_from(ids.len()).map_err(|_| TooManyFiles)?;
        let mut uf = UnionFind { ids, count, parent: (0..count).collect(), size: vec![1; index(count)] };
        for &(a, b) in edges {
            let (a, b) = (uf.position(a)?, uf.position(b)?);
            uf.union(a, b);
        }
        Ok(uf)
    }

    /// The number of `id`: its position in the sorted ids. Every id an edge names is there, so the search finds it.
    fn position(&self, id: FileId) -> Result<u32, TooManyFiles> {
        let at = self.ids.binary_search(&id).unwrap_or_else(|at| at);
        u32::try_from(at).map_err(|_| TooManyFiles)
    }

    fn parent(&self, x: u32) -> u32 {
        self.parent[index(x)]
    }

    /// The root of `x`'s group. Halves the path as it goes, so a later find from the same file is short.
    fn root(&mut self, mut x: u32) -> u32 {
        loop {
            let p = self.parent(x);
            if p == x {
                return x;
            }
            let grandparent = self.parent(p);
            self.parent[index(x)] = grandparent;
            x = grandparent;
        }
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return;
        }
        let (sa, sb) = (self.size[index(ra)], self.size[index(rb)]);
        let (big, small) = if sa >= sb { (ra, rb) } else { (rb, ra) };
        self.parent[index(small)] = big;
        self.size[index(big)] = sa + sb;
    }

    /// The groups of two or more files, each listing its files in id order, largest first. The groups are made in the
    /// order of their smallest ids, and the sort keeps that order between groups of the same size.
    pub fn groups(&mut self) -> Vec<Vec<FileId>> {
        let mut group_of_root: Vec<Option<usize>> = vec![None; index(self.count)];
        let mut groups: Vec<Vec<FileId>> = Vec::new();
        for x in 0..self.count {
            let root = index(self.root(x));
            let group = *group_of_root[root].get_or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group].push(self.ids[index(x)]);
        }
        groups.retain(|g| g.len() > 1);
        groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_chain_of_links_stays_shallow_and_is_one_group() {
        // Each edge joins a file to the one above it, from the top down, so the chain grows by one link each time. Joining
        // the larger group under the smaller id's made a chain 200,000 deep; the smaller group joins the larger here, so
        // every file is at most log₂ n links from its root.
        let edges: Vec<(FileId, FileId)> = (1..=200_000).rev().map(|i| (FileId::new(i - 1), FileId::new(i))).collect();
        let mut uf = UnionFind::from_edges(&edges).unwrap();
        let deepest = (0..uf.count)
            .map(|mut x| {
                let mut depth = 0;
                while uf.parent(x) != x {
                    x = uf.parent(x);
                    depth += 1;
                }
                depth
            })
            .max();
        assert!(deepest.is_some_and(|d| d <= 18), "{deepest:?}");
        let groups = uf.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 200_001);
    }

    #[test]
    fn groups_are_the_connected_files_of_two_or_more_in_id_order_largest_first() {
        let id = FileId::new;
        let edges = [(id(9), id(4)), (id(4), id(7)), (id(2), id(1)), (id(5), id(5)), (id(20), id(30))];
        let groups = UnionFind::from_edges(&edges).unwrap().groups();
        assert_eq!(groups, [vec![id(4), id(7), id(9)], vec![id(1), id(2)], vec![id(20), id(30)]]);
    }
}
