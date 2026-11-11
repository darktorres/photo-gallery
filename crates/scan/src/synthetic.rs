//! A synthetic folder tree for the benches (`crate::scan_with`, behind the `bench` feature): a listing with no file
//! system under it, the same tree on every run, so a walk's instruction count is the walk's own.

use std::collections::BTreeMap;
use std::io;

use pg_model::name::Name;
use pg_model::refusal::RefusedReason;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::dir::Entry;
use pg_platform::names::units_of;
use pg_platform::native::Unit;

use crate::Listing;

/// One entry of a synthetic folder.
struct SyntheticEntry {
    name: Vec<Unit>,
    folder: bool,
    file_id: u64,
    size: u64,
    mtime: i64,
}

/// A library folder holding `folders` subfolders of `files` media files each, and `unreadable` more subfolders that
/// can't be opened (each one a row of the folder ledger). Every entry is on volume 1, and its name, file ID, size and
/// time are fixed by its place in the tree.
pub struct SyntheticTree {
    /// Each folder that can be opened, by its units as the walk opens it: its listing's index in `listings`.
    folders: BTreeMap<Vec<Unit>, usize>,
    /// The entries of each folder that can be opened, the root's first.
    listings: Vec<Vec<SyntheticEntry>>,
}

impl SyntheticTree {
    /// The tree under `root` (a folder of `rules`, as the catalog walked has it).
    pub fn new(rules: PathRules, root: &RulePath, folders: usize, files: usize, unreadable: usize) -> SyntheticTree {
        let mut next_id: u64 = 1;
        let mut id = || {
            let this = next_id;
            next_id += 1;
            this
        };
        let mut map = BTreeMap::new();
        map.insert(units_of(&rules.join(root)), 0);
        let (mut top, mut listings) = (Vec::new(), Vec::new());
        for f in 0..folders.saturating_add(unreadable) {
            let name = format!("folder-{f:05}");
            top.push(SyntheticEntry {
                name: units_of(name.as_bytes()),
                folder: true,
                file_id: id(),
                size: 0,
                mtime: 0,
            });
            if f >= folders {
                // Listed by its parent, never opened.
                continue;
            }
            let path = root.child(&Name::from(name.as_str()));
            map.insert(units_of(&rules.join(&path)), listings.len() + 1);
            let entries = (0..files)
                .map(|i| {
                    let file_id = id();
                    SyntheticEntry {
                        name: units_of(format!("img-{i:05}.jpg").as_bytes()),
                        folder: false,
                        file_id,
                        size: 100_000 + file_id,
                        mtime: 1_700_000_000 + i64::try_from(file_id).unwrap_or(i64::MAX - 1_700_000_000),
                    }
                })
                .collect();
            listings.push(entries);
        }
        listings.insert(0, top);
        SyntheticTree { folders: map, listings }
    }
}

impl Listing for SyntheticTree {
    type Dir = usize;

    fn open(&mut self, path: &[Unit]) -> io::Result<usize> {
        self.folders.get(path).copied().ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "unreadable"))
    }

    fn volume(&mut self, _dir: &usize) -> io::Result<u64> {
        Ok(1)
    }

    fn each(&mut self, dir: &usize, mut each: impl FnMut(Entry<'_>)) -> io::Result<()> {
        let listing = self.listings.get(*dir).ok_or_else(|| io::Error::other("no such synthetic folder"))?;
        for e in listing {
            each(Entry {
                name: &e.name,
                folder: e.folder,
                reparse: false,
                file_id: e.file_id,
                size: e.size,
                mtime: e.mtime,
                volume: None,
                created: None,
            });
        }
        Ok(())
    }

    fn check_mount(&mut self, _path: &[Unit]) -> Result<(), RefusedReason> {
        Ok(())
    }

    fn fallback_volumes(&self) -> u64 {
        0
    }
}
