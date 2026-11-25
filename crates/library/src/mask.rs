//! Which of a lane's passes a round runs. The app runs them all; a driver (`pg-dev`, the tests) can run a chosen set, and the
//! passes it leaves out are skipped in the real round, not run some other way. The set is held in a cell the job reads at
//! the start of each round, so a change applies from the next round.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// One of a lane's passes, in the order a round runs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassKind {
    /// The folders asked for are walked.
    Scans,
    /// The pictures are analyzed from their headers.
    Import,
    /// The thumbnails at the grid's height are made.
    Thumbs,
    /// The thumbnail placeholders are made.
    Placeholders,
    /// The duplicate hashes are made.
    Hashes,
    /// The duplicates are paired.
    Pairing,
    /// The borders are looked for.
    Borders,
}

impl PassKind {
    /// Every pass, in round order.
    pub const ALL: [PassKind; 7] = [
        PassKind::Scans,
        PassKind::Import,
        PassKind::Thumbs,
        PassKind::Placeholders,
        PassKind::Hashes,
        PassKind::Pairing,
        PassKind::Borders,
    ];

    /// The bit of this pass in a [`PassMask`].
    fn bit(self) -> u8 {
        match self {
            PassKind::Scans => 1,
            PassKind::Import => 1 << 1,
            PassKind::Thumbs => 1 << 2,
            PassKind::Placeholders => 1 << 3,
            PassKind::Hashes => 1 << 4,
            PassKind::Pairing => 1 << 5,
            PassKind::Borders => 1 << 6,
        }
    }
}

/// A set of passes: the ones a round runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassMask(u8);

impl PassMask {
    /// Every pass: what the app's rounds run.
    pub const ALL: PassMask = PassMask(0b111_1111);

    /// No pass at all.
    pub const NONE: PassMask = PassMask(0);

    /// The passes of `kinds`, and no others.
    pub fn only(kinds: &[PassKind]) -> PassMask {
        PassMask(kinds.iter().fold(0, |bits, kind| bits | kind.bit()))
    }

    /// Whether `kind` is in the set.
    pub fn contains(self, kind: PassKind) -> bool {
        self.0 & kind.bit() != 0
    }
}

impl Default for PassMask {
    fn default() -> PassMask {
        PassMask::ALL
    }
}

/// The pass set the job's rounds read, shared between the runner and its lanes.
#[derive(Clone, Debug)]
pub(crate) struct PassCell(Arc<AtomicU8>);

impl PassCell {
    /// A cell holding `mask`.
    pub(crate) fn new(mask: PassMask) -> PassCell {
        PassCell(Arc::new(AtomicU8::new(mask.0)))
    }

    /// The set, as it is now.
    pub(crate) fn get(&self) -> PassMask {
        PassMask(self.0.load(Ordering::SeqCst))
    }

    /// Replace the set. A round in progress keeps the set it began with.
    pub(crate) fn set(&self, mask: PassMask) {
        self.0.store(mask.0, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mask_holds_exactly_the_passes_it_names() {
        assert!(PassKind::ALL.iter().all(|&kind| PassMask::ALL.contains(kind)));
        assert!(PassKind::ALL.iter().all(|&kind| !PassMask::NONE.contains(kind)));
        let some = PassMask::only(&[PassKind::Thumbs, PassKind::Placeholders]);
        for kind in PassKind::ALL {
            let named = matches!(kind, PassKind::Thumbs | PassKind::Placeholders);
            assert_eq!(some.contains(kind), named, "{kind:?}");
        }
    }

    #[test]
    fn every_pass_has_its_own_bit() {
        let bits: Vec<u8> = PassKind::ALL.iter().map(|kind| kind.bit()).collect();
        let union = bits.iter().fold(0, |all, bit| all | bit);
        assert_eq!(union, PassMask::ALL.0);
        assert_eq!(usize::try_from(union.count_ones()).ok(), Some(bits.len()));
    }

    #[test]
    fn the_cell_gives_back_what_was_set() {
        let cell = PassCell::new(PassMask::ALL);
        assert_eq!(cell.get(), PassMask::ALL);
        cell.set(PassMask::only(&[PassKind::Borders]));
        assert_eq!(cell.get(), PassMask::only(&[PassKind::Borders]));
    }
}
