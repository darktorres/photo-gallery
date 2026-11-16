//! The background features the owner can switch off in the Library panel: borders detection, and duplicate hashing with
//! its pairing. Their values are the preferences' (`pg_app`'s `Prefs::features`); the job and the import take them as
//! data, so the model is their one definition.

/// Which background features are on. Each defaults to on (the preferences' default); there is no `Default` impl, so a
/// zeroed value is never taken for "all off" by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// Border detection (`pg_borders`): the Borders pass runs when on.
    pub borders: bool,
    /// Duplicate hashing and pairing: the import computes the file and pixel hashes and PDQ when on, and the Hashes and
    /// Pairing passes run.
    pub duplicates: bool,
}

impl Features {
    pub const ALL: Features = Features { borders: true, duplicates: true };

    /// What the import and the Hashes pass do about the duplicate hashes.
    pub const fn hashing(self) -> Hashing {
        if self.duplicates { Hashing::Compute } else { Hashing::Skip }
    }
}

/// Whether an analysis computes the duplicate hashes (the file's BLAKE3, the pixels' XXH3 and PDQ). A value of its own,
/// not a `bool`, so a call site reads what it asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hashing {
    Compute,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_duplicates_feature_decides_the_hashing() {
        assert_eq!(Features::ALL.hashing(), Hashing::Compute);
        assert_eq!(Features { borders: true, duplicates: false }.hashing(), Hashing::Skip);
    }
}
