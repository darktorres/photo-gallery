//! The background features the owner can switch off in the Library panel: borders detection, and duplicate hashing with
//! its pairing. Their values are the preferences' (`pg-app`'s `Prefs::features`); the job and the import take them as
//! data, so the model is their one definition.

/// Which background features are on. Each defaults to on (the preferences' default); there is no `Default` impl, so a
/// zeroed value is never taken for "all off" by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// Border detection (`pg-borders`): the Borders pass runs when on.
    pub borders: bool,
    /// Duplicate hashing and pairing: the Hashes pass computes the file and pixel hashes and PDQ when on, and the
    /// Pairing pass runs. The import itself hashes nothing (it reads headers only), so the hashes wait for this.
    pub duplicates: bool,
}

impl Features {
    /// The features on as the app starts: borders and duplicates.
    pub const ALL: Features = Features { borders: true, duplicates: true };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_features_are_on() {
        assert_eq!(Features::ALL, Features { borders: true, duplicates: true });
    }
}
