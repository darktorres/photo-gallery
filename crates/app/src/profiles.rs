//! `drive-profiles.json`: the reader counts the job's lanes learned per disk and pass kind (`pg_model::profile`), kept in
//! a file of its own beside the preferences, not in them (machine-learned data, deleted at will: a missing or unreadable
//! file is no profile, and the lanes start from their drive kinds' priors). Atomic and quarantined as the preferences
//! are (`JsonStore`). The library defines the store's interface (`pg_library::tuning::ProfileStore`); this is the app's.

use std::collections::BTreeMap;
use std::path::PathBuf;

use pg_library::tuning::ProfileStore;
use pg_model::profile::{DriveProfiles, ProfileEntry};

use crate::store::JsonStore;

/// The file's name, in the layout's home folder.
pub const FILE: &str = "drive-profiles.json";

/// The app's profile store, over its file.
pub struct JsonProfiles {
    store: JsonStore<DriveProfiles>,
}

impl JsonProfiles {
    /// The store of the file at `path`. A file that doesn't read is kept aside and nothing is loaded: the profiles are a
    /// hint, so no notice is posted for it.
    pub fn open(path: PathBuf) -> JsonProfiles {
        let (store, _unreadable) = JsonStore::open(path);
        JsonProfiles { store }
    }
}

impl ProfileStore for JsonProfiles {
    fn load(&self) -> DriveProfiles {
        self.store.get()
    }

    fn merge(&self, entries: BTreeMap<String, ProfileEntry>) -> std::io::Result<()> {
        self.store.update(|profiles| profiles.entries.extend(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(readers: u32) -> ProfileEntry {
        ProfileEntry {
            kind: "solid".into(),
            readers,
            tested_min: 1,
            tested_max: 8,
            mb_s: 400.0,
            samples: 300,
            at: 5,
            os: "windows".into(),
        }
    }

    #[test]
    fn merged_entries_are_kept_in_the_file_with_the_others() {
        let dir = pg_testkit::run_folder("scratch_app", "profiles").unwrap();
        let path = dir.join(FILE);
        let profiles = JsonProfiles::open(path.clone());
        assert!(profiles.load().entries.is_empty(), "no file: no profile");
        profiles.merge(BTreeMap::from([("a/import".to_string(), entry(2))])).unwrap();
        profiles.merge(BTreeMap::from([("b/thumbs".to_string(), entry(4))])).unwrap();
        let reopened = JsonProfiles::open(path);
        let entries = reopened.load().entries;
        assert_eq!(entries.len(), 2, "the second merge kept the first");
        assert_eq!(entries.get("a/import").map(|e| e.readers), Some(2));
    }

    #[test]
    fn an_unreadable_file_is_no_profile() {
        let dir = pg_testkit::run_folder("scratch_app", "profiles-bad").unwrap();
        let path = dir.join(FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(JsonProfiles::open(path).load().entries.is_empty());
    }
}
