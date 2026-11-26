//! What `Catalog::prepare` reports while it works: the phase it is in, and the bytes of a copy. It is report only: a
//! `Progress` can't stop the work. Quitting in the middle of a change is safe because every step is crash-atomic, and the
//! next start resumes from the file's `user_version`. There is no cancel, and no SQLite progress handler.

/// The free bytes of a volume, as a test reports them in place of the volume's own (`Prepare::free_space`). Defined here,
/// a leaf module, so the room check (`lifecycle`) and the builder (`core`) both name it without a cycle between them.
pub type FreeSpace<'a> = dyn Fn(&std::path::Path) -> Option<u64> + Send + Sync + 'a;

/// A phase of preparing a catalog, in the order a change makes them. The variants the change's later steps make (a copy
/// aside, its checkpoint and fsync, the swap, the vacuum, the quick check) are defined here so the report's shape is
/// fixed once; each is reported by the step that makes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Judging the file: its version, its schema and its path rules. Nothing is written yet.
    Check,
    /// Copying the catalog aside before a migration changes it. A rebuild's copy is [`Phase::Copy`].
    Backup,
    /// Step `step` (0-based) of `steps` of the migration that takes the catalog from `from` to `to`.
    Migrate {
        /// The schema version the migration starts from.
        from: i64,
        /// The schema version it takes the catalog to.
        to: i64,
        /// The step, from 0.
        step: usize,
        /// How many steps the migration has.
        steps: usize,
    },
    /// Carrying the old catalog's rows into a new layout (a rebuild).
    Carry,
    /// Merging the write-ahead log into the file before a copy or a swap.
    Checkpoint,
    /// Flushing a copy to disk before it is renamed into place.
    Fsync,
    /// Copying the file aside for a rebuild, beside the old catalog. The copy taken before a migration is [`Phase::Backup`].
    Copy,
    /// Putting the new file in the old one's place.
    Swap,
    /// Rewriting the file to reclaim its free pages (the compaction an earlier change owes).
    Vacuum,
    /// Checking the file's integrity after a change.
    QuickCheck,
}

/// Receives a preparation's progress. The methods return nothing, and the work does not wait for them.
pub trait Progress: Send + Sync {
    /// The preparation has entered `phase`.
    fn phase(&self, _phase: Phase) {}

    /// A copy has done `done` of `total` bytes (`total` is zero when it is not known).
    fn bytes(&self, _done: u64, _total: u64) {}
}

/// Reports nothing: the progress of a preparation nobody watches.
pub struct NoProgress;

impl Progress for NoProgress {}

/// [`NoProgress`], as a static, for the places that take a reference to one.
pub(crate) static NO_PROGRESS: NoProgress = NoProgress;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Records the phases in order.
    struct Recorder(Mutex<Vec<Phase>>);

    impl Progress for Recorder {
        fn phase(&self, phase: Phase) {
            if let Ok(mut phases) = self.0.lock() {
                phases.push(phase);
            }
        }
    }

    #[test]
    fn the_default_methods_do_nothing_and_a_recorder_keeps_the_order() {
        NoProgress.phase(Phase::Check);
        NoProgress.bytes(1, 2);
        let recorder = Recorder(Mutex::new(Vec::new()));
        recorder.phase(Phase::Check);
        recorder.phase(Phase::Migrate { from: 1, to: 2, step: 0, steps: 3 });
        recorder.bytes(5, 0);
        assert_eq!(
            *recorder.0.lock().unwrap(),
            vec![Phase::Check, Phase::Migrate { from: 1, to: 2, step: 0, steps: 3 }]
        );
    }

    use std::path::Path;

    use pg_platform::native::PATH_RULES;

    use crate::Catalog;
    use crate::schema::{BASELINE, Migration, SchemaSet, Step};
    use crate::test_support::versions;

    /// The steps of the migration the tests make: two tables, so a report names each step.
    static EXTRA_STEPS: &[Step] =
        &[Step::Sql("CREATE TABLE extra (x INTEGER)"), Step::Sql("CREATE TABLE extra2 (x INTEGER)")];
    static ONE_MIGRATION: &[Migration] = &[Migration { steps: EXTRA_STEPS, rebuilds_tables: false }];
    /// The baseline and that one migration: a catalog at the baseline migrates to version 2 under it.
    const WITH_EXTRA: SchemaSet<'static> = SchemaSet { baseline: BASELINE, migrations: ONE_MIGRATION };

    fn phases(recorder: &Recorder) -> Vec<Phase> {
        recorder.0.lock().map(|phases| phases.clone()).unwrap_or_default()
    }

    /// A fresh catalog is made at the baseline, with nothing to migrate: the only phase reported is the check.
    #[test]
    fn a_fresh_catalog_reports_only_the_check() {
        let dir = pg_testkit::run_folder("scratch_catalog", "fresh").unwrap();
        let recorder = Recorder(Mutex::new(Vec::new()));
        let catalog =
            Catalog::preparing(&dir.join("catalog.sqlite"), versions(), PATH_RULES).progress(&recorder).run().unwrap();
        drop(catalog);
        assert_eq!(phases(&recorder), vec![Phase::Check]);
    }

    /// A migration reports the copy aside of the catalog it migrates, then each of its steps in order.
    #[test]
    fn a_migration_reports_its_backup_and_each_step() {
        let dir = pg_testkit::run_folder("scratch_catalog", "migrate").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PATH_RULES).unwrap());
        let recorder = Recorder(Mutex::new(Vec::new()));
        let catalog =
            Catalog::preparing(&path, versions(), PATH_RULES).schemas(&WITH_EXTRA).progress(&recorder).run().unwrap();
        let tables: i64 = catalog
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('extra', 'extra2')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 2, "both steps ran");
        drop(catalog);
        assert!(crate::rebuild::backup_path(&path, 1).exists(), "the catalog as it was is kept");
        let migrate = |step| Phase::Migrate { from: 1, to: 2, step, steps: 2 };
        assert_eq!(phases(&recorder), vec![Phase::Check, Phase::Backup, migrate(0), migrate(1)]);
    }

    /// A migration is refused before anything is copied when its volume has no room: the catalog is left as it was.
    #[test]
    fn a_migration_is_refused_before_the_copy_when_the_volume_has_no_room() {
        let dir = pg_testkit::run_folder("scratch_catalog", "room").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PATH_RULES).unwrap());
        let no_room = |_: &Path| Some(0_u64);
        let refused = Catalog::preparing(&path, versions(), PATH_RULES).schemas(&WITH_EXTRA).free_space(&no_room).run();
        assert!(refused.is_err(), "no room is refused");
        let message = refused.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(message.contains("free some space"), "{message}");
        assert!(!crate::rebuild::backup_path(&path, 1).exists(), "nothing was copied aside");
        assert!(Catalog::open(&path, PATH_RULES).is_ok(), "the catalog is still at the baseline");
    }
}
