//! The pass's work lists: paths kept end to end in arenas of bytes ([`PathArena`], [`WorkList`]), each named by its range.
//! What a path is, how it orders and where it is contained is in `rules.rs` ([`crate::rules::PathRules`]); this module
//! only holds the bytes a pass reads and the paths it hands out. The wide form a file is opened by is the platform's
//! (`PathRules::to_wide`), built from a work list's folder and name.

use std::collections::BTreeMap;

/// Paths kept end to end in one buffer of bytes, each named by its range ([`PathRef`]). It grows by doubling, so a
/// list of n paths makes about log₂ n allocations. Bytes, not text: a path is stored as the OS has it (`Name` bytes
/// joined by the separator, `PathRules::join`), which need not be UTF-8.
#[derive(Default, Debug)]
pub struct PathArena {
    bytes: Vec<u8>,
}

/// A path in a [`PathArena`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PathRef {
    start: usize,
    end: usize,
}

impl PathArena {
    /// `path` added at the end.
    pub fn push(&mut self, path: &[u8]) -> PathRef {
        self.push_parts(&[path])
    }

    /// The concatenation of `parts`, added at the end.
    pub fn push_parts(&mut self, parts: &[&[u8]]) -> PathRef {
        let start = self.bytes.len();
        for part in parts {
            self.bytes.extend_from_slice(part);
        }
        PathRef { start, end: self.bytes.len() }
    }

    /// `path`'s bytes; empty for a path of another arena that reaches past this one's.
    pub fn get(&self, path: PathRef) -> &[u8] {
        self.bytes.get(path.start..path.end).unwrap_or_default()
    }
}

/// A file of a [`WorkList`]: its folder's path and its name, in the list's arenas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FilePath {
    folder: PathRef,
    name: PathRef,
}

/// A pass's work: its items, each holding its path as a [`FilePath`]. Each folder's path is held once, read the
/// first time one of its files is added ([`WorkList::file`]).
pub struct WorkList<T> {
    /// The list's items, in the order they were added.
    pub items: Vec<T>,
    folders: PathArena,
    /// Each folder id's path in `folders`, once read. A map, so its size is what the list holds, not the largest id.
    folder_paths: BTreeMap<i64, PathRef>,
    names: PathArena,
}

impl<T> WorkList<T> {
    /// Room for `items` items.
    pub fn with_capacity(items: usize) -> Self {
        WorkList {
            items: Vec::with_capacity(items),
            folders: PathArena::default(),
            folder_paths: BTreeMap::new(),
            names: PathArena::default(),
        }
    }

    /// The path of the file `name` (its name's bytes) in folder `folder_id`. The folder's path is read by
    /// `read_folder` (into the arena it is given) the first time it is asked for.
    pub fn file<E>(
        &mut self,
        folder_id: i64,
        name: &[u8],
        read_folder: impl FnOnce(&mut PathArena) -> Result<PathRef, E>,
    ) -> Result<FilePath, E> {
        let folder = match self.folder_paths.get(&folder_id).copied() {
            Some(folder) => folder,
            None => {
                let folder = read_folder(&mut self.folders)?;
                self.folder_paths.insert(folder_id, folder);
                folder
            }
        };
        Ok(FilePath { folder, name: self.names.push(name) })
    }

    /// The file's folder's path and its name, as bytes.
    pub fn parts(&self, path: FilePath) -> [&[u8]; 2] {
        [self.folders.get(path.folder), self.names.get(path.name)]
    }

    /// Its path, joined by `sep` (the separator of its rules, `PathRules::sep`), for the programs it is handed to
    /// and for messages through the rules' display.
    pub fn path(&self, path: FilePath, sep: u8) -> Vec<u8> {
        let [folder, name] = self.parts(path);
        [folder, &[sep][..], name].concat()
    }

    /// How many files the list holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list holds no files.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> Default for WorkList<T> {
    fn default() -> Self {
        WorkList::with_capacity(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arena_gives_back_each_path() {
        let mut arena = PathArena::default();
        let refs: Vec<PathRef> =
            (0..1000).map(|i| arena.push(format!(r"D:\folder {i}\file {i}.jpg").as_bytes())).collect();
        let joined = arena.push_parts(&[br"E:\copies", br"\a.jpg"]);
        for (i, r) in refs.iter().enumerate() {
            assert_eq!(arena.get(*r), format!(r"D:\folder {i}\file {i}.jpg").as_bytes());
        }
        assert_eq!(arena.get(joined), br"E:\copies\a.jpg");
        let empty = arena.push(b"");
        assert_eq!(arena.get(empty), b"");
    }

    #[test]
    fn a_work_list_reads_each_folder_once() {
        let mut list = WorkList::<()>::with_capacity(4);
        let mut reads = 0;
        let mut add = |list: &mut WorkList<()>, folder: i64, name: &str| {
            list.file(folder, name.as_bytes(), |arena| {
                reads += 1;
                Ok::<_, ()>(arena.push(format!(r"D:\f{folder}").as_bytes()))
            })
            .unwrap()
        };
        let a = add(&mut list, 2, "a.jpg");
        let b = add(&mut list, 0, "b.jpg");
        let c = add(&mut list, 2, "c.jpg");
        // Past the size it was given.
        let d = add(&mut list, 9, "d.jpg");
        assert_eq!(reads, 3);
        assert_eq!(
            [a, b, c, d].map(|p| list.path(p, b'\\')),
            [r"D:\f2\a.jpg", r"D:\f0\b.jpg", r"D:\f2\c.jpg", r"D:\f9\d.jpg"].map(|s| s.as_bytes().to_vec())
        );
        assert_eq!(list.parts(c), [r"D:\f2".as_bytes(), b"c.jpg"]);
    }

    /// A folder id far above the folders there are (after removals, ids are never reused) costs the list one entry, not
    /// a table as long as the id.
    #[test]
    fn a_large_folder_id_costs_one_entry() {
        let mut list = WorkList::<()>::default();
        let mut read = 0;
        let path = list
            .file(1_000_000_000, b"a.jpg", |arena| {
                read += 1;
                Ok::<_, ()>(arena.push(br"D:\far"))
            })
            .unwrap();
        assert_eq!(read, 1);
        assert_eq!(list.parts(path), [br"D:\far".as_slice(), b"a.jpg"]);
        assert_eq!(list.folder_paths.len(), 1);
    }
}
