//! What follows a scan, the same for the window's job and the command line: the replaced files' thumbnails forgotten
//! (their temps are `pg_writeback::discard_temps`'s), and the notices its result carries.

use pg_model::ids::FileId;
use pg_model::notice::{Notice, ScanNote};
use pg_model::rules::Path as RulePath;
use pg_scan::ScanStats;
use pg_thumbcache::ThumbCache;

/// Forget the thumbnails of the files a scan's pairing replaced (`ScanStats::replaced_ids`, decision 7), after the
/// scan's commit. The ones that couldn't be, with why.
pub fn forget_replaced(thumbs: &ThumbCache, ids: &[FileId]) -> Vec<(FileId, anyhow::Error)> {
    ids.iter().filter_map(|&id| thumbs.forget(id).err().map(|e| (id, e))).collect()
}

/// The notices a scan of `root` carries in `stats`: its unreadable folders, the mounts it refused, and its notes
/// (`ScanNote`, each with its count; a zero is not said). Its kept failures (the files it skipped) are not notices.
pub fn scan_notices(root: &RulePath, stats: &ScanStats) -> Vec<Notice> {
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let mut found = Vec::new();
    if stats.errors > 0 {
        found.push(Notice::ScanUnreadable { root: root.clone(), count: stats.errors });
    }
    for refused in &stats.refused_mount_list {
        found.push(Notice::MountRefused {
            root: root.clone(),
            mount: refused.mount.clone(),
            reason: refused.reason.clone(),
        });
    }
    let notes = [
        (ScanNote::Duplicates, stats.duplicates),
        (ScanNote::SkippedReparse, stats.skipped_reparse),
        (ScanNote::Ambiguous, count(stats.pairing.ambiguous)),
        (ScanNote::UnpairedWithData, count(stats.pairing.unpaired_with_data)),
        (ScanNote::Replaced, count(stats.pairing.replaced)),
        (ScanNote::Rebound, count(stats.pairing.rebound)),
        (ScanNote::VolumeFallback, stats.volume_fallbacks),
    ];
    for (note, count) in notes {
        if count > 0 {
            found.push(Notice::ScanNote { root: root.clone(), note, count });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use pg_model::refusal::RefusedReason;
    use pg_scan::RefusedMount;

    use super::*;

    /// Every notice a scan's result carries, and only those: a zero count says nothing.
    #[test]
    fn a_scans_result_carries_its_unreadable_folders_refused_mounts_and_notes() {
        let rules = pg_model::rules::PathRules::Posix;
        let root = rules.parse("/photos").unwrap();
        let mount = rules.parse("/photos/usb").unwrap();
        let stats = ScanStats {
            errors: 2,
            refused_mount_list: vec![RefusedMount {
                mount: mount.clone(),
                reason: RefusedReason::NotNtfs { file_system: "vfat".into() },
            }],
            duplicates: 3,
            volume_fallbacks: 1,
            ..ScanStats::default()
        };
        let notices = scan_notices(&root, &stats);
        assert_eq!(
            notices,
            [
                Notice::ScanUnreadable { root: root.clone(), count: 2 },
                Notice::MountRefused {
                    root: root.clone(),
                    mount,
                    reason: RefusedReason::NotNtfs { file_system: "vfat".into() }
                },
                Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 3 },
                Notice::ScanNote { root, note: ScanNote::VolumeFallback, count: 1 },
            ]
        );
        assert!(scan_notices(&rules.parse("/a").unwrap(), &ScanStats::default()).is_empty());
    }
}
