//! The reader counts the lanes learned, per physical disk and per pass kind: the content of `drive-profiles.json`, a file
//! of its own beside the preferences (machine-learned data, which can be deleted at any time). A stored count only seeds a
//! lane's tuner, which still checks it (`pg_jobs::tuner::Prior::of_profile`); an entry that is old, of another kind of
//! drive, or learned from too little is not trusted at all ([`ProfileEntry::trusted`]).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

use serde::{Deserialize, Serialize};

/// The longest an entry is trusted: 30 days.
pub const TRUSTED_FOR_SECS: i64 = 30 * 24 * 60 * 60;

/// The fewest reads an entry is trusted with.
pub const TRUSTED_SAMPLES: u64 = 200;

/// The file's content: its entries by key ([`DriveProfiles::key`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveProfiles {
    /// The entries, by disk and pass kind.
    pub entries: BTreeMap<String, ProfileEntry>,
}

/// What one lane learned for one pass kind on one disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileEntry {
    /// The drive's kind when it was learned: `rotational`, `solid` or `unknown`.
    pub kind: String,
    /// The stable reader count.
    pub readers: u32,
    /// The lowest count measured.
    pub tested_min: u32,
    /// The highest count measured.
    pub tested_max: u32,
    /// The throughput at `readers`, in MB/s.
    pub mb_s: f64,
    /// The reads of the read-bound windows it was learned from.
    pub samples: u64,
    /// When it was learned, in Unix seconds.
    pub at: i64,
    /// The system it was learned on (`windows`, `linux`).
    pub os: String,
}

impl DriveProfiles {
    /// The key of a disk's entry for a pass kind: the disk's identity (`pg_platform::drive::Drive::identity`) in hex,
    /// then the pass.
    pub fn key(identity: u64, pass: &str) -> String {
        format!("{identity:016x}/{pass}")
    }
}

impl ProfileEntry {
    /// Whether the entry may seed a tuner now (`now`, Unix seconds) on a drive of `kind`: no older than 30 days, of the
    /// same kind, and learned from enough reads.
    pub fn trusted(&self, kind: &str, now: i64) -> bool {
        self.kind == kind && now.saturating_sub(self.at) <= TRUSTED_FOR_SECS && self.samples >= TRUSTED_SAMPLES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> ProfileEntry {
        ProfileEntry {
            kind: "rotational".into(),
            readers: 1,
            tested_min: 1,
            tested_max: 4,
            mb_s: 178.0,
            samples: 500,
            at: 1_000_000,
            os: "windows".into(),
        }
    }

    #[test]
    fn an_entry_is_trusted_while_young_of_its_kind_and_well_sampled() {
        let e = entry();
        assert!(e.trusted("rotational", 1_000_000 + TRUSTED_FOR_SECS));
        assert!(!e.trusted("rotational", 1_000_001 + TRUSTED_FOR_SECS), "too old");
        assert!(!e.trusted("solid", 1_000_000), "another kind of drive");
        assert!(!ProfileEntry { samples: 10, ..entry() }.trusted("rotational", 1_000_000), "too few reads");
    }

    #[test]
    fn the_key_is_the_identity_in_hex_and_the_pass() {
        assert_eq!(DriveProfiles::key(0xab, "import"), "00000000000000ab/import");
    }

    #[test]
    fn the_file_round_trips_and_a_missing_field_reads_as_empty() {
        let mut profiles = DriveProfiles::default();
        profiles.entries.insert(DriveProfiles::key(1, "thumbs"), entry());
        let text = serde_json::to_string(&profiles).unwrap();
        assert_eq!(serde_json::from_str::<DriveProfiles>(&text).unwrap(), profiles);
        assert_eq!(serde_json::from_str::<DriveProfiles>("{}").unwrap(), DriveProfiles::default());
    }
}
