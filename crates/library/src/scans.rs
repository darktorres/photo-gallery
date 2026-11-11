//! What follows a scan, the same for the window's job and the command line: the replaced files' thumbnails forgotten
//! (their temps are `pg_writeback::discard_temps`'s), and the notices its result carries.

use pg_model::ids::FileId;
use pg_model::notice::{NoteLifecycle, Notice, ScanNote};
use pg_model::rules::Path as RulePath;
use pg_scan::ScanStats;
use pg_thumbcache::ThumbCache;

/// Forget the thumbnails of the files a scan's pairing replaced (`ScanStats::replaced_ids`, decision 7), after the
/// scan's commit. The ones that couldn't be, with why.
pub fn forget_replaced(thumbs: &ThumbCache, ids: &[FileId]) -> Vec<(FileId, anyhow::Error)> {
    ids.iter().filter_map(|&id| thumbs.forget(id).err().map(|e| (id, e))).collect()
}

/// The notices a scan's result carries, split by what each describes ([`ScanNote::lifecycle`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanNotices {
    /// The conditions of the library folder (hard links, reparse points, ambiguous rows…): kept under `(root, code)`,
    /// and replaced only by a complete walk of the whole library folder, which is what holds for it.
    pub conditions: Vec<Notice>,
    /// What this walk did (rows replaced, rows bound again): posted by the walk that did it, whatever folder it walked.
    pub events: Vec<Notice>,
}

impl ScanNotices {
    /// Every notice, the conditions first.
    pub fn all(&self) -> impl Iterator<Item = &Notice> {
        self.conditions.iter().chain(&self.events)
    }
}

/// The notices a scan of `root` carries in `stats`: its notes (`ScanNote`, each with its count; a zero is not said).
/// The folders it couldn't read and the mounts it refused are not notices (the catalog's folder ledger has them), nor
/// are its kept failures (the files it skipped).
pub fn scan_notices(root: &RulePath, stats: &ScanStats) -> ScanNotices {
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let notes = [
        (ScanNote::Duplicates, stats.duplicates),
        (ScanNote::SkippedReparse, stats.skipped_reparse),
        (ScanNote::Ambiguous, count(stats.pairing.ambiguous)),
        (ScanNote::UnpairedWithData, count(stats.pairing.unpaired_with_data)),
        (ScanNote::Replaced, count(stats.pairing.replaced)),
        (ScanNote::Rebound, count(stats.pairing.rebound)),
        (ScanNote::VolumeFallback, stats.volume_fallbacks),
    ];
    let mut found = ScanNotices::default();
    for (note, count) in notes {
        if count == 0 {
            continue;
        }
        let notice = Notice::ScanNote { root: root.clone(), note, count };
        match note.lifecycle() {
            NoteLifecycle::Condition => found.conditions.push(notice),
            NoteLifecycle::Event => found.events.push(notice),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use pg_model::refusal::RefusedReason;
    use pg_scan::RefusedMount;

    use super::*;

    /// Every note a scan's result carries, and only those (a zero count says nothing), split into the folder's
    /// conditions and the walk's events. The unreadable folders and the refused mounts are the folder ledger's.
    #[test]
    fn a_scans_result_carries_its_notes_split_into_conditions_and_events() {
        let rules = pg_model::rules::PathRules::Posix;
        let root = rules.parse("/photos").unwrap();
        let mount = rules.parse("/photos/usb").unwrap();
        let mut stats = ScanStats {
            errors: 2,
            refused_mounts: 1,
            refused_mount_list: vec![RefusedMount {
                mount,
                reason: RefusedReason::NotNtfs { file_system: "vfat".into() },
            }],
            duplicates: 3,
            volume_fallbacks: 1,
            ..ScanStats::default()
        };
        stats.pairing.rebound = 4;
        let notices = scan_notices(&root, &stats);
        let note = |note, count| Notice::ScanNote { root: root.clone(), note, count };
        assert_eq!(notices.conditions, [note(ScanNote::Duplicates, 3), note(ScanNote::VolumeFallback, 1)]);
        assert_eq!(notices.events, [note(ScanNote::Rebound, 4)]);
        assert_eq!(notices.all().count(), 3);
        assert_eq!(scan_notices(&rules.parse("/a").unwrap(), &ScanStats::default()), ScanNotices::default());
    }

    /// The split is the model's table: every note is one or the other.
    #[test]
    fn every_note_is_a_condition_or_an_event() {
        let rules = pg_model::rules::PathRules::Posix;
        let root = rules.parse("/photos").unwrap();
        let mut stats = ScanStats { duplicates: 1, skipped_reparse: 1, volume_fallbacks: 1, ..ScanStats::default() };
        stats.pairing.ambiguous = 1;
        stats.pairing.unpaired_with_data = 1;
        stats.pairing.replaced = 1;
        stats.pairing.rebound = 1;
        let notices = scan_notices(&root, &stats);
        assert_eq!(notices.all().count(), ScanNote::ALL.len());
        let events: Vec<ScanNote> =
            ScanNote::ALL.into_iter().filter(|n| n.lifecycle() == NoteLifecycle::Event).collect();
        assert_eq!(events, [ScanNote::Replaced, ScanNote::Rebound]);
        assert_eq!(notices.events.len(), events.len());
    }
}
