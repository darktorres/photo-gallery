//! Quarter turns clockwise: a file's rotation as the catalog keeps it (`files.rotation`), and a rotation still queued
//! for it (`file_writes`). A whole turn of four is none, so a value is always 0 to 3.

/// Quarter turns clockwise, 0 to 3. Serialized as its number of turns; a signed count (`-1` is three turns, the
/// counterclockwise one) reads as the same turns, so a recording of the signed form replays.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "i64", into = "u8")]
pub struct Turns(u8);

impl From<i64> for Turns {
    fn from(n: i64) -> Turns {
        Turns::of(n)
    }
}

impl From<Turns> for u8 {
    fn from(turns: Turns) -> u8 {
        turns.0
    }
}

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

#[cfg(test)]
mod tests {
    use super::Turns;

    #[test]
    fn a_signed_count_reads_as_its_turns_and_is_written_as_a_bare_number() {
        assert_eq!(serde_json::from_str::<Turns>("-1").unwrap(), Turns::new(3));
        assert_eq!(serde_json::from_str::<Turns>("5").unwrap(), Turns::new(1));
        assert_eq!(serde_json::to_string(&Turns::new(2)).unwrap(), "2");
        assert_eq!(
            serde_json::from_str::<Turns>(&serde_json::to_string(&Turns::new(3)).unwrap()).unwrap(),
            Turns::new(3)
        );
    }
}
