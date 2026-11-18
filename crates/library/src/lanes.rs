//! Which library roots share a lane. A lane is a thread of the background job that walks and processes its roots' files;
//! the lanes run at once, one per physical drive, so folders on two drives make progress together, and folders on one
//! drive go one after the other, since two readers on one spindle fight over its head.
//!
//! This module decides the grouping, from each root's drive (`pg_platform::drive`). It is pure, so it is tested without
//! a disk: the job asks the drives, then groups them here.

use std::collections::BTreeMap;

use pg_catalog::Catalog;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::drive::{Drive, DriveKey, SystemDrives, same_drive};
use pg_platform::native::PATH_RULES;

/// A root's drive, when it could be told. `None` is an unknown drive.
pub type RootDrive = Option<Drive>;

/// What a root's probe found: its volume and drive, that its drive is not there, or that it can't be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// On a volume (as the catalog stores it), on a drive that could be told.
    Mounted { volume: i64, drive: Drive },
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
pub fn plan(roots: &[(RulePath, Option<Place>)], serial: bool) -> Vec<Vec<RulePath>> {
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
    let lane_of = assign(&drives, serial);
    let count = lane_of.iter().copied().max().map_or(0, |most| most + 1);
    let mut lanes: Vec<Vec<RulePath>> = vec![Vec::new(); count];
    for ((root, _), lane) in mounted.iter().zip(lane_of) {
        if let Some(members) = lanes.get_mut(lane) {
            members.push((*root).clone());
        }
    }
    lanes
}

/// One lane's work for a command that runs the lanes itself (the CLI's): its roots, the volumes they are on (`All` when the
/// lane is the only one, as in the job), and its drive's label for people (`E:`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub roots: Vec<RulePath>,
    pub volumes: pg_catalog::VolumeSet,
    pub label: String,
}

/// The groups of `roots`, as the job would make its lanes (`plan`), probed now with the system's drives on the calling
/// thread. A root that is unplugged, or whose drive can't be told, is in no group (a command that is about to fail says
/// so). With `serial`, every root is in one group.
pub fn groups(rules: PathRules, roots: &[RulePath], serial: bool) -> Vec<Group> {
    let probes: Vec<(RulePath, Option<Place>)> = roots
        .iter()
        .map(|root| (root.clone(), Some(crate::coordinator::probe_place(rules, root, &SystemDrives))))
        .collect();
    let planned = plan(&probes, serial);
    let single = planned.len() == 1;
    planned
        .into_iter()
        .map(|members| {
            let place_of =
                |root: &RulePath| probes.iter().find(|(path, _)| path == root).and_then(|(_, place)| place.clone());
            let label = members
                .first()
                .and_then(|root| match place_of(root) {
                    Some(Place::Mounted { drive, .. }) => Some(drive.label),
                    _ => None,
                })
                .unwrap_or_default();
            let volumes = if single {
                pg_catalog::VolumeSet::All
            } else {
                pg_catalog::VolumeSet::Only(
                    members
                        .iter()
                        .filter_map(|root| match place_of(root) {
                            Some(Place::Mounted { volume, .. }) => Some(volume),
                            _ => None,
                        })
                        .collect(),
                )
            };
            Group { roots: members, volumes, label }
        })
        .collect()
}

/// The lanes a command runs over a catalog: [`groups`] of its roots, or, when it has no roots, one lane over every volume
/// (what a command with no folders has always run). `serial` is the escape hatch (one lane).
pub fn command_groups(catalog: &Catalog, serial: bool) -> anyhow::Result<Vec<Group>> {
    let roots: Vec<RulePath> = catalog.roots()?.into_iter().map(|r| r.path).collect();
    let groups = groups(catalog.rules(), &roots, serial);
    if groups.is_empty() {
        return Ok(vec![Group { roots: Vec::new(), volumes: pg_catalog::VolumeSet::All, label: String::new() }]);
    }
    Ok(groups)
}

/// Runs `pass` once per group, each on a connection of its own (opened at `catalog_path`) and a child of `ctl`
/// (`Control::lane`), one thread per group, so the drives are worked at once. The results are in the groups' order. A lane
/// that panics fails the run. The run's cores are shared among the lanes, as the job shares them (`Control::share_cpu`).
pub fn run_each<T: Send>(
    catalog_path: &std::path::Path,
    groups: &[Group],
    ctl: &pg_jobs::control::Control,
    pass: impl Fn(&mut Catalog, &Group, &pg_jobs::control::Control) -> anyhow::Result<T> + Sync,
) -> anyhow::Result<Vec<T>> {
    ctl.share_cpu(std::thread::available_parallelism().map_or(4, |n| n.get()));
    let lanes: Vec<(&Group, std::sync::Arc<pg_jobs::control::Control>)> =
        groups.iter().map(|group| (group, ctl.lane())).collect();
    std::thread::scope(|scope| {
        let pass = &pass;
        let handles: Vec<_> = lanes
            .iter()
            .map(|(group, lane)| {
                scope.spawn(move || -> anyhow::Result<T> {
                    let mut catalog = Catalog::open(catalog_path, PATH_RULES)?;
                    pass(&mut catalog, group, lane)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_else(|_| Err(anyhow::anyhow!("a lane panicked"))))
            .collect()
    })
}

/// The lane of each root, by index: roots on the same drive share a number, and the numbers run from 0 in the order the
/// drives first appear. Two roots share a lane when their disks overlap (`same_drive`), so a volume spanning two disks
/// and a volume on one of them are one lane.
///
/// Fails closed: a root whose drive is unknown puts every root in lane 0, the one-lane behaviour, since its disks could
/// be any other root's. `serial` (the escape hatch for setups the probe can't see through: two disk images on one
/// spindle, Storage Spaces) does the same by choice.
pub fn assign(drives: &[RootDrive], serial: bool) -> Vec<usize> {
    if serial || drives.iter().any(Option::is_none) {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pg_platform::drive::Kind;

    use super::*;

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
        assert_eq!(assign(&[disks(&[1]), disks(&[2])], false), vec![0, 1]);
    }

    #[test]
    fn roots_on_one_drive_share_a_lane_and_lanes_number_from_the_first_root() {
        assert_eq!(assign(&[disks(&[1]), disks(&[2]), disks(&[1])], false), vec![0, 1, 0]);
    }

    #[test]
    fn a_spanned_volume_and_a_volume_on_one_of_its_disks_are_one_lane() {
        assert_eq!(assign(&[disks(&[1, 2]), disks(&[3]), disks(&[2])], false), vec![0, 1, 0]);
    }

    #[test]
    fn an_unknown_drive_puts_every_root_in_one_lane() {
        assert_eq!(assign(&[disks(&[1]), None, disks(&[2])], false), vec![0, 0, 0]);
    }

    #[test]
    fn serial_puts_every_root_in_one_lane_even_on_different_drives() {
        assert_eq!(assign(&[disks(&[1]), disks(&[2])], true), vec![0, 0]);
    }

    #[test]
    fn opaque_roots_share_one_lane_and_are_apart_from_physical_disks() {
        assert_eq!(assign(&[opaque(), disks(&[1]), opaque()], false), vec![0, 1, 0]);
    }

    #[test]
    fn no_roots_is_no_lanes() {
        assert!(assign(&[], false).is_empty());
    }

    fn plan_of(roots: &[(RulePath, Option<Place>)]) -> Vec<Vec<RulePath>> {
        plan(roots, false)
    }

    fn root(path: &str) -> RulePath {
        pg_model::rules::PathRules::Posix.parse(path).unwrap()
    }

    fn mounted(volume: i64, numbers: &[u64]) -> Option<Place> {
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
