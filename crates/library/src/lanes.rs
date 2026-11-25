//! Which library roots share a lane. A lane is a thread of the background job that walks and processes its roots' files;
//! the lanes run at once, one per physical drive, so folders on two drives make progress together, and folders on one
//! drive go one after the other, since two readers on one spindle fight over its head.
//!
//! This module decides the grouping, from each root's drive (`pg_platform::drive`). The grouping itself (`plan`, `assign`,
//! `place_on_volume`) is pure, so it is tested without a disk. The probes that ask the disks are `probe_place` (the
//! root's volume through `pg_scan::root_volume`, its drive through a `Drives`) and `groups`, which uses the system's
//! drives; they run on the calling thread, and then their answers are grouped here.

use std::collections::BTreeMap;

use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::drive::{Drive, DriveKey, Drives, same_drive, unplugged};

/// A root's drive, when it could be told. `None` is an unknown drive.
pub type RootDrive = Option<Drive>;

/// What a root's probe found: its volume and drive, that its drive is not there, or that it can't be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// On a volume (as the catalog stores it), on a drive that could be told.
    Mounted {
        /// The volume's number, as the catalog stores it.
        volume: u64,
        /// The drive the volume is on, as it could be told.
        drive: Drive,
    },
    /// Not there (unplugged, not ready, the path gone): no lane until it is back.
    Unplugged,
    /// Can't be told, or the probe ran past its deadline: fails closed.
    Unknown,
}

/// The lanes the roots make, each as its roots in order, the lanes in the order of their first root (`assign`).
///
/// A root whose place is [`Place::Unknown`] puts every other root that is not unplugged in one lane: fail closed, the
/// one-lane behaviour, since the unknown root's files could be on any drive. A root that isn't probed yet (`None`) has no
/// lane until it is, unless an unknown root collapses the rest. An unplugged root has none at all.
pub fn plan(roots: &[(RulePath, Option<Place>)]) -> Vec<Vec<RulePath>> {
    if roots.iter().any(|(_, place)| matches!(place, Some(Place::Unknown))) {
        let all: Vec<RulePath> = roots
            .iter()
            .filter(|(_, place)| !matches!(place, Some(Place::Unplugged)))
            .map(|(root, _)| root.clone())
            .collect();
        return if all.is_empty() { Vec::new() } else { vec![all] };
    }
    let mounted: Vec<(&RulePath, &Drive)> = roots
        .iter()
        .filter_map(|(root, place)| match place {
            Some(Place::Mounted { drive, .. }) => Some((root, drive)),
            _ => None,
        })
        .collect();
    let drives: Vec<RootDrive> = mounted.iter().map(|(_, drive)| Some((*drive).clone())).collect();
    let lane_of = assign(&drives);
    let count = lane_of.iter().copied().max().map_or(0, |most| most + 1);
    let mut lanes: Vec<Vec<RulePath>> = vec![Vec::new(); count];
    for ((root, _), lane) in mounted.iter().zip(lane_of) {
        if let Some(members) = lanes.get_mut(lane) {
            members.push((*root).clone());
        }
    }
    lanes
}

/// The volume set of a lane: every volume when it is the only lane, else the volumes its roots are on. The one rule the
/// planner and the coordinator both apply.
pub(crate) fn lane_volumes(single: bool, volumes: impl Iterator<Item = u64>) -> pg_catalog::VolumeSet {
    if single { pg_catalog::VolumeSet::All } else { pg_catalog::VolumeSet::Only(volumes.collect()) }
}

/// The drive for people of a lane's roots: the drive of the first of them whose place is known (`E:`), empty when none
/// is. The one rule the planner and the coordinator both apply.
pub(crate) fn drive_label<'a>(
    roots: impl IntoIterator<Item = &'a RulePath>,
    place_of: impl Fn(&RulePath) -> Option<Place>,
) -> String {
    roots
        .into_iter()
        .find_map(|root| match place_of(root) {
            Some(Place::Mounted { drive, .. }) => Some(drive.label),
            _ => None,
        })
        .unwrap_or_default()
}

/// The lane of each root, by index: roots on the same drive share a number, and the numbers run from 0 in the order the
/// drives first appear. Two roots share a lane when their disks overlap (`same_drive`), so a volume spanning two disks
/// and a volume on one of them are one lane.
///
/// Fails closed: a root whose drive is unknown puts every root in lane 0, the one-lane behaviour, since its disks could
/// be any other root's. A setup the probe can't see through (an opaque device on Linux and the real disk under it) gets
/// a lane per device: that gap is accepted (`docs/reference/STORAGE.md`).
pub fn assign(drives: &[RootDrive]) -> Vec<usize> {
    if drives.iter().any(Option::is_none) {
        return vec![0; drives.len()];
    }
    let keys: Vec<DriveKey> = drives.iter().flatten().map(|drive| drive.key.clone()).collect();
    let firsts = same_drive(&keys);
    let mut lane_of: BTreeMap<usize, usize> = BTreeMap::new();
    firsts
        .into_iter()
        .map(|first| {
            let next = lane_of.len();
            *lane_of.entry(first).or_insert(next)
        })
        .collect()
}

/// Where `root` is, as the lanes need it: its volume (as the catalog stores it) and its drive, or that it is not there, or
/// that it can't be told.
pub(crate) fn probe_place(rules: PathRules, root: &RulePath, drives: &dyn Drives) -> Place {
    let folder = pg_platform::names::disk_path(rules, root);
    let volume = match pg_scan::root_volume(rules, root) {
        Ok(volume) => volume,
        Err(e) => return if unplugged(&e) { Place::Unplugged } else { Place::Unknown },
    };
    let drive = match drives.of(&folder) {
        Ok(drive) => drive,
        Err(e) => return if unplugged(&e) { Place::Unplugged } else { Place::Unknown },
    };
    place_on_volume(volume, drive)
}

/// A root on `volume` (the id the file system gives) and `drive`. The id is held as the file system gives it; the catalog
/// stores its bits as a signed number at the SQL edge (`cast_signed`), so an id above `i64::MAX` is no reason to take the
/// root as unknown.
fn place_on_volume(volume: u64, drive: Drive) -> Place {
    Place::Mounted { volume, drive }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pg_platform::drive::Kind;

    use super::*;

    #[test]
    fn a_volume_id_above_i64_max_is_mounted_as_the_catalog_stores_it() {
        let drive = Drive { key: DriveKey::Opaque, kind: Kind::Unknown, label: "virtual".to_string() };
        assert!(matches!(place_on_volume(u64::MAX, drive), Place::Mounted { volume: u64::MAX, .. }));
    }

    fn disks(numbers: &[u64]) -> RootDrive {
        Some(Drive {
            key: DriveKey::Disks(numbers.iter().copied().collect::<BTreeSet<u64>>()),
            kind: Kind::Rotational,
            label: format!("{numbers:?}"),
        })
    }

    fn opaque() -> RootDrive {
        Some(Drive { key: DriveKey::Opaque, kind: Kind::Unknown, label: "virtual".to_string() })
    }

    #[test]
    fn roots_on_different_drives_are_different_lanes() {
        assert_eq!(assign(&[disks(&[1]), disks(&[2])]), vec![0, 1]);
    }

    #[test]
    fn roots_on_one_drive_share_a_lane_and_lanes_number_from_the_first_root() {
        assert_eq!(assign(&[disks(&[1]), disks(&[2]), disks(&[1])]), vec![0, 1, 0]);
    }

    #[test]
    fn a_spanned_volume_and_a_volume_on_one_of_its_disks_are_one_lane() {
        assert_eq!(assign(&[disks(&[1, 2]), disks(&[3]), disks(&[2])]), vec![0, 1, 0]);
    }

    #[test]
    fn an_unknown_drive_puts_every_root_in_one_lane() {
        assert_eq!(assign(&[disks(&[1]), None, disks(&[2])]), vec![0, 0, 0]);
    }

    #[test]
    fn opaque_roots_share_one_lane_and_are_apart_from_physical_disks() {
        assert_eq!(assign(&[opaque(), disks(&[1]), opaque()]), vec![0, 1, 0]);
    }

    #[test]
    fn no_roots_is_no_lanes() {
        assert!(assign(&[]).is_empty());
    }

    fn plan_of(roots: &[(RulePath, Option<Place>)]) -> Vec<Vec<RulePath>> {
        plan(roots)
    }

    fn root(path: &str) -> RulePath {
        pg_model::rules::PathRules::Posix.parse(path).unwrap()
    }

    fn mounted(volume: u64, numbers: &[u64]) -> Option<Place> {
        let drive = disks(numbers).unwrap();
        Some(Place::Mounted { volume, drive })
    }

    #[test]
    fn a_plan_has_one_lane_per_drive_in_the_order_of_their_roots() {
        let roots = [(root("/a"), mounted(1, &[1])), (root("/b"), mounted(2, &[2])), (root("/c"), mounted(3, &[1]))];
        assert_eq!(plan_of(&roots), vec![vec![root("/a"), root("/c")], vec![root("/b")]]);
    }

    #[test]
    fn an_unplugged_root_has_no_lane_and_the_others_keep_theirs() {
        let roots = [(root("/a"), mounted(1, &[1])), (root("/b"), Some(Place::Unplugged))];
        assert_eq!(plan_of(&roots), vec![vec![root("/a")]]);
    }

    #[test]
    fn a_root_not_probed_yet_has_no_lane_until_it_is() {
        let roots = [(root("/a"), mounted(1, &[1])), (root("/b"), None)];
        assert_eq!(plan_of(&roots), vec![vec![root("/a")]]);
    }

    #[test]
    fn an_unknown_root_collapses_every_root_that_is_not_unplugged_into_one_lane() {
        let roots = [
            (root("/a"), mounted(1, &[1])),
            (root("/b"), Some(Place::Unknown)),
            (root("/c"), mounted(3, &[2])),
            (root("/d"), Some(Place::Unplugged)),
            (root("/e"), None),
        ];
        assert_eq!(plan_of(&roots), vec![vec![root("/a"), root("/b"), root("/c"), root("/e")]]);
    }

    #[test]
    fn no_root_is_no_lane_when_all_are_unplugged() {
        assert!(plan_of(&[(root("/a"), Some(Place::Unplugged))]).is_empty());
        assert!(plan_of(&[]).is_empty());
    }
}
