//! The import's tier-1 pairs. A new picture's 8 orientations are queried against the index and then inserted, so every
//! pair is found once, whichever picture came first.

use pg_model::ids::FileId;
use pg_model::pdq::{Hash, MATCH_DISTANCE};

use crate::pdq_index::{IndexFull, PdqIndex};

/// The index the import pairs new pictures against, with the buffers a query keeps from one picture to the next.
pub struct Pairer {
    index: PdqIndex,
    /// One orientation's neighbours, as the index finds them.
    found: Vec<(FileId, u32)>,
    /// Each neighbour's best (distance, orientation variant), for the picture being paired.
    best: Vec<(FileId, u32, usize)>,
}

impl Pairer {
    /// The index of the stored hashes: `ids` and their hash words, as the crate's index takes them (`PdqIndex::build`).
    pub fn build(ids: Vec<FileId>, hashes: Vec<[u64; 4]>) -> Pairer {
        Pairer { index: PdqIndex::build(ids, hashes), found: Vec::new(), best: Vec::new() }
    }

    /// The number of hashes stored.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether no hash is stored.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Room for `additional` more orientations' hashes.
    pub fn reserve(&mut self, additional: usize) {
        self.index.reserve(additional);
    }

    /// Query the picture `id`'s orientations against the index, then insert them. The result is each other picture's
    /// best (distance, orientation variant) within [`MATCH_DISTANCE`]; the picture's own entries are left out.
    /// [`IndexFull`], and none of them inserted, when the index has no room for them.
    pub fn pair_and_insert(&mut self, id: FileId, orientations: &[Hash]) -> Result<&[(FileId, u32, usize)], IndexFull> {
        self.index.best_matches(orientations, MATCH_DISTANCE, &mut self.found, &mut self.best);
        self.best.retain(|&(other, _, _)| other != id);
        self.index.insert_orientations(id, orientations)?;
        Ok(&self.best)
    }
}
