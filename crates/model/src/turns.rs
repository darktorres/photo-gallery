//! Quarter turns clockwise: a file's rotation as the catalog keeps it (`files.rotation`), and a rotation still queued
//! for it (`file_writes`). A whole turn of four is none, so a value is always 0 to 3.

/// Quarter turns clockwise, 0 to 3.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Turns(u8);

impl Turns {
    /// No turn.
    pub const NONE: Turns = Turns(0);

    /// `n` quarter turns, taken modulo four.
    pub const fn new(n: u8) -> Turns {
        Turns(n % 4)
    }

    /// `n` quarter turns clockwise, or anticlockwise when negative: any count, either way, modulo four.
    pub const fn of(n: i64) -> Turns {
        // `rem_euclid(4)` is 0 to 3 (`try_from` isn't const).
        Turns(match n.rem_euclid(4) {
            1 => 1,
            2 => 2,
            3 => 3,
            _ => 0,
        })
    }

    /// The number of quarter turns, 0 to 3.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Whether a quarter turn (one or three) swaps the picture's width and height.
    pub const fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }

    /// The turns after `self` and `other` (quarter turns add up, a whole turn being none).
    pub const fn plus(self, other: Turns) -> Turns {
        Turns::new(self.0 + other.0)
    }
}
