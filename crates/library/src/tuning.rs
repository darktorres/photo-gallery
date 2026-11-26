//! A lane's reader tuning: one tuner (`pg_jobs::tuner`) per pass kind that reads files, kept by the lane across its
//! passes, so a tuner's warm-up, hold-downs and probes carry from pass to pass and it converges. The lane is one drive (a
//! connected group of physical disks, `lanes`), so this is tuning per drive; its binding to the drive is set by the
//! coordinator, and a lane whose drive changes starts its tuners again.
//!
//! A tuner starts from what the lane learned before on that drive, when it is trusted (`pg_model::profile`), or else from
//! the drive kind's prior. What a lane learns is kept in a [`ProfileStore`] (the app's `drive-profiles.json`), written
//! when the lane goes idle or is retired, for the pass kinds whose tuner is stable and was read-bound long enough. A
//! drive with no identity (no serial number, or opaque) has no profile; a pinned lane writes none.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pg_jobs::tuner::{Learned, Prior, Tuner, TunerConfig};
use pg_model::profile::{DriveProfiles, ProfileEntry};
use pg_platform::drive::{Drive, DriveKey, Kind};

/// Told one line for each change of a lane's reader count (`RunnerConfig::on_tuning`).
pub type TuningLog = Arc<dyn Fn(&str) + Send + Sync>;

/// Where a run keeps what its lanes learned: the app's file, or a test's. The library defines it, the app implements it
/// (the library cannot depend on the app's `JsonStore`), as the drives are injected.
pub trait ProfileStore: Send + Sync {
    /// The profiles stored now (none, when there is no file or it doesn't read).
    fn load(&self) -> DriveProfiles;
    /// Stores `entries` over the stored ones, keeping the others: one merge, so two lanes writing at once lose nothing.
    fn merge(&self, entries: BTreeMap<String, ProfileEntry>) -> std::io::Result<()>;
}

/// The pass kinds a lane tunes: the passes that read files through the pipeline's readers. Pairing reads none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TunedPass {
    /// The import: headers and gathers.
    Import,
    /// The thumbnails: whole files, decoded.
    Thumbs,
    /// The ThumbHash placeholders.
    Placeholders,
    /// The duplicate hashes: whole files.
    Hashes,
    /// The borders: whole files, decoded.
    Borders,
}

impl TunedPass {
    /// Its name, in the profile's keys and the log.
    pub fn name(self) -> &'static str {
        match self {
            TunedPass::Import => "import",
            TunedPass::Thumbs => "thumbs",
            TunedPass::Placeholders => "placeholders",
            TunedPass::Hashes => "hashes",
            TunedPass::Borders => "borders",
        }
    }
}

/// A profile is written for a pass kind whose read-bound windows lasted this long...
pub const WRITE_AFTER: Duration = Duration::from_secs(60);
/// ...or read this many files.
pub const WRITE_AFTER_READS: u64 = 200;

/// The drive a lane is bound to, as its tuners need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    /// The drive's disks now (they change at a restart, the identity does not).
    pub(crate) key: DriveKey,
    pub(crate) kind: Kind,
    pub(crate) identity: Option<u64>,
}

impl Binding {
    pub(crate) fn of(drive: &Drive) -> Binding {
        Binding { key: drive.key.clone(), kind: drive.kind, identity: drive.identity }
    }
}

/// The name of a drive kind, as a profile stores it.
pub(crate) fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Rotational => "rotational",
        Kind::Solid => "solid",
        Kind::Unknown => "unknown",
    }
}

// The time now, in Unix seconds: the platform's clock, which every crate reads the same way.
pub(crate) use pg_platform::clock::unix_now as now_secs;

/// A lane's tuners and its drive. See the module comment.
pub(crate) struct LaneTuner {
    cfg: TunerConfig,
    binding: Option<Binding>,
    tuners: BTreeMap<TunedPass, Arc<Mutex<Tuner>>>,
    /// What was written last, so an idle lane that learned nothing new writes nothing.
    written: BTreeMap<String, ProfileEntry>,
}

impl LaneTuner {
    pub(crate) fn new(cfg: TunerConfig) -> LaneTuner {
        LaneTuner { cfg, binding: None, tuners: BTreeMap::new(), written: BTreeMap::new() }
    }

    /// Binds the lane to `binding`: when it differs from the one before, the tuners start again (a kept lane's drive can
    /// change). Whether it changed.
    pub(crate) fn bind(&mut self, binding: Option<Binding>) -> bool {
        if self.binding == binding {
            return false;
        }
        self.binding = binding;
        self.tuners.clear();
        self.written.clear();
        true
    }

    /// The lane's tuner for `pass`: the one it has, or a new one seeded from `store`'s trusted profile for the drive at
    /// `now`, or from the drive kind's prior (unknown before the lane is bound).
    pub(crate) fn tuner(&mut self, pass: TunedPass, store: Option<&dyn ProfileStore>, now: i64) -> Arc<Mutex<Tuner>> {
        if let Some(tuner) = self.tuners.get(&pass) {
            return tuner.clone();
        }
        let kind = self.binding.as_ref().map_or(Kind::Unknown, |b| b.kind);
        let stored = self
            .binding
            .as_ref()
            .and_then(|b| b.identity)
            .zip(store)
            .and_then(|(identity, store)| store.load().entries.remove(&DriveProfiles::key(identity, pass.name())))
            .filter(|entry| entry.trusted(kind_name(kind), now));
        let prior =
            match stored.and_then(|e| std::num::NonZeroUsize::new(usize::try_from(e.readers).ok()?).zip(Some(e))) {
                Some((readers, entry)) => Prior::of_profile(readers, entry.mb_s * 1e6, kind),
                None => Prior::of_kind(kind),
            };
        let tuner = Arc::new(Mutex::new(Tuner::new(self.cfg, prior)));
        self.tuners.insert(pass, tuner.clone());
        tuner
    }

    /// The entries this lane would write now: the pass kinds whose tuner is stable and was read-bound for
    /// [`WRITE_AFTER`] or [`WRITE_AFTER_READS`], on a drive with an identity. Empty otherwise.
    pub(crate) fn entries(&self, now: i64) -> BTreeMap<String, ProfileEntry> {
        let Some(binding) = &self.binding else { return BTreeMap::new() };
        let Some(identity) = binding.identity else { return BTreeMap::new() };
        self.tuners
            .iter()
            .filter_map(|(pass, tuner)| {
                let learned = tuner.lock().unwrap_or_else(PoisonError::into_inner).learned();
                let enough = learned.read_bound >= WRITE_AFTER || learned.read_bound_reads >= WRITE_AFTER_READS;
                (learned.stable && enough)
                    .then(|| (DriveProfiles::key(identity, pass.name()), entry(&learned, binding.kind, now)))
            })
            .collect()
    }

    /// Writes what the lane learned into `store`, when it learned something since its last write.
    pub(crate) fn write(&mut self, store: &dyn ProfileStore, now: i64) -> std::io::Result<()> {
        let entries = self.entries(now);
        let new: BTreeMap<String, ProfileEntry> = entries
            .into_iter()
            .filter(|(key, entry)| {
                self.written.get(key).is_none_or(|old| old.readers != entry.readers || old.samples != entry.samples)
            })
            .collect();
        if new.is_empty() {
            return Ok(());
        }
        store.merge(new.clone())?;
        self.written.extend(new);
        Ok(())
    }
}

/// A profile entry of what a tuner learned on a drive of `kind`, at `now`.
fn entry(learned: &Learned, kind: Kind, now: i64) -> ProfileEntry {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    ProfileEntry {
        kind: kind_name(kind).to_string(),
        readers: count(learned.readers.get()),
        tested_min: count(learned.tested.0),
        tested_max: count(learned.tested.1),
        mb_s: learned.bytes_per_sec / 1e6,
        samples: learned.read_bound_reads,
        at: now,
        os: std::env::consts::OS.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::num::NonZeroUsize;

    use pg_jobs::tuner::Sample;

    use super::*;

    /// A store in memory.
    #[derive(Default)]
    struct Memory(Mutex<DriveProfiles>);

    impl ProfileStore for Memory {
        fn load(&self) -> DriveProfiles {
            self.0.lock().unwrap().clone()
        }

        fn merge(&self, entries: BTreeMap<String, ProfileEntry>) -> std::io::Result<()> {
            self.0.lock().unwrap().entries.extend(entries);
            Ok(())
        }
    }

    fn binding(kind: Kind, identity: Option<u64>) -> Option<Binding> {
        Some(Binding { key: DriveKey::Disks(BTreeSet::from([1])), kind, identity })
    }

    /// A read-bound window of 3 s at 100 MB/s, whatever the count.
    fn flat() -> Sample {
        Sample {
            elapsed: Duration::from_secs(3),
            reads: 40,
            bytes: 300_000_000,
            disk_ns: 40 * 5_000_000,
            work_wait_ns: 6_000_000_000,
            workers: 4,
            active_readers: 2.0,
            files: 40,
            ..Sample::default()
        }
    }

    #[test]
    fn a_tuner_starts_from_its_kinds_prior_and_is_kept_across_passes() {
        let mut lane = LaneTuner::new(TunerConfig::default());
        lane.bind(binding(Kind::Solid, Some(7)));
        let first = lane.tuner(TunedPass::Import, None, 0);
        assert_eq!(first.lock().unwrap().target().get(), 4, "the solid prior");
        let again = lane.tuner(TunedPass::Import, None, 0);
        assert!(Arc::ptr_eq(&first, &again), "one tuner per pass kind, kept");
        assert!(!Arc::ptr_eq(&first, &lane.tuner(TunedPass::Thumbs, None, 0)));
    }

    #[test]
    fn a_new_drive_starts_the_tuners_again() {
        let mut lane = LaneTuner::new(TunerConfig::default());
        lane.bind(binding(Kind::Solid, Some(7)));
        let first = lane.tuner(TunedPass::Import, None, 0);
        assert!(!lane.bind(binding(Kind::Solid, Some(7))), "the same drive changes nothing");
        assert!(Arc::ptr_eq(&first, &lane.tuner(TunedPass::Import, None, 0)));
        assert!(lane.bind(binding(Kind::Rotational, Some(8))));
        let second = lane.tuner(TunedPass::Import, None, 0);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.lock().unwrap().target().get(), 2, "the rotational prior");
    }

    #[test]
    fn a_stable_read_bound_tuner_is_written_and_seeds_the_next_lane() {
        let store = Memory::default();
        let mut lane = LaneTuner::new(TunerConfig::default());
        lane.bind(binding(Kind::Rotational, Some(7)));
        let tuner = lane.tuner(TunedPass::Import, Some(&store), 1000);
        // Past the minimum read-bound time, until a window leaves it stable (not mid-probe).
        for window in 0..400 {
            tuner.lock().unwrap().update(&flat());
            if window >= 40 && tuner.lock().unwrap().learned().stable {
                break;
            }
        }
        assert!(tuner.lock().unwrap().learned().stable);
        lane.write(&store, 1000).unwrap();
        let stored = store.load();
        let entry = stored.entries.get(&DriveProfiles::key(7, "import")).expect("written");
        assert_eq!(entry.kind, "rotational");
        // A lane on the same drive starts from it.
        let mut next = LaneTuner::new(TunerConfig::default());
        next.bind(binding(Kind::Rotational, Some(7)));
        let seeded = next.tuner(TunedPass::Import, Some(&store), 1000);
        assert_eq!(
            seeded.lock().unwrap().target(),
            NonZeroUsize::new(usize::try_from(entry.readers).unwrap()).unwrap()
        );
    }

    #[test]
    fn nothing_is_written_for_a_drive_without_an_identity_or_a_short_pass() {
        let store = Memory::default();
        let mut lane = LaneTuner::new(TunerConfig::default());
        lane.bind(binding(Kind::Rotational, None));
        let tuner = lane.tuner(TunedPass::Import, Some(&store), 0);
        for _ in 0..60 {
            tuner.lock().unwrap().update(&flat());
        }
        lane.write(&store, 0).unwrap();
        assert!(store.load().entries.is_empty(), "no identity, no profile");
        let mut short = LaneTuner::new(TunerConfig::default());
        short.bind(binding(Kind::Rotational, Some(7)));
        short.tuner(TunedPass::Import, Some(&store), 0).lock().unwrap().update(&flat());
        short.write(&store, 0).unwrap();
        assert!(store.load().entries.is_empty(), "a pass too short teaches nothing");
    }

    #[test]
    fn an_untrusted_profile_seeds_nothing() {
        let store = Memory::default();
        let old = ProfileEntry {
            kind: "rotational".into(),
            readers: 8,
            tested_min: 1,
            tested_max: 8,
            mb_s: 100.0,
            samples: 1000,
            at: 0,
            os: "windows".into(),
        };
        store.merge(BTreeMap::from([(DriveProfiles::key(7, "import"), old)])).unwrap();
        let mut lane = LaneTuner::new(TunerConfig::default());
        lane.bind(binding(Kind::Rotational, Some(7)));
        let now = pg_model::profile::TRUSTED_FOR_SECS + 1;
        assert_eq!(
            lane.tuner(TunedPass::Import, Some(&store), now).lock().unwrap().target().get(),
            2,
            "too old: the prior"
        );
    }
}
