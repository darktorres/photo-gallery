//! Windows paths as the catalog and the tree compare them (model-shaped: pure string and byte work, no catalog or
//! Windows calls): the merged path of a folder (its drive dropped, so every drive's copy is one folder), the byte
//! keys that order folders and files by name (case-insensitive, exact after that, so the order is total), and
//! whether a path is inside another. The pass's work lists (paths kept end to end in arenas, [`WorkList`]) are here
//! too: the paths they hold are what the catalog hands out.

use crate::paths::verbatim_wide;

/// A folder's path without its drive: `E:\pinterest\a` and `D:\pinterest\a` are both `pinterest\a`; a drive
/// root is the empty path. Other paths (UNC) are kept whole.
pub fn merged_path(path: &str) -> &str {
    match path.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => path[2..].trim_start_matches('\\'),
        _ => path,
    }
}

/// The components of a merged path, as the tree has them: `\\server\share\x` is `server`, `share`, `x`.
pub(crate) fn components(merged: &str) -> impl Iterator<Item = &str> {
    merged.split('\\').filter(|p| !p.is_empty())
}

/// The order key's part for a folder: each component of its merged path in lower case (`str::to_lowercase`, a
/// final Σ as ς), then 0x01. Bytes 0x01 and 0x02 in a name (NTFS allows them, Win32 doesn't) are escaped as
/// 0x02 0x02 and 0x02 0x03, which keeps the order. Compared as bytes, these keys order folders component by
/// component, as the tree does: `p`, `p\sub`, `p-x`; the same folder on every drive is one key.
pub fn folder_key(merged: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(merged.len() + 4);
    push_folder_key(&mut key, merged);
    key
}

/// [`folder_key`] appended to `key`.
pub fn push_folder_key(key: &mut Vec<u8>, merged: &str) {
    fn escaped(key: &mut Vec<u8>, bytes: &[u8]) {
        for &b in bytes {
            match b {
                0x01 => key.extend([0x02, 0x02]),
                0x02 => key.extend([0x02, 0x03]),
                _ => key.push(b),
            }
        }
    }
    let mut buf = [0; 4];
    for part in components(merged) {
        // Character by character, which is `str::to_lowercase` for every character but Σ: its lower case depends
        // on where it stands (a final one is ς), so a component with one is lowercased whole.
        if part.contains('Σ') {
            escaped(key, part.to_lowercase().as_bytes());
        } else {
            for c in part.chars().flat_map(char::to_lowercase) {
                escaped(key, c.encode_utf8(&mut buf).as_bytes());
            }
        }
        key.push(0x01);
    }
}

/// The order key's part for a file name: 0x00, the name in lower case character by character, 0x00, the name
/// itself. Compared as bytes (UTF-8 keeps code-point order): case-insensitive (Unicode, unlike SQLite's NOCASE),
/// then exact, so the order is total. The leading 0x00 puts a folder's files before its subfolders.
pub fn name_key(name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(name_key_len(name));
    push_name_key(&mut key, name);
    key
}

/// [`name_key`] appended to `key`.
pub fn push_name_key(key: &mut Vec<u8>, name: &str) {
    key.push(0x00);
    let mut buf = [0; 4];
    for c in name.chars().flat_map(char::to_lowercase) {
        key.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    key.push(0x00);
    key.extend_from_slice(name.as_bytes());
}

/// How long [`name_key`] is.
fn name_key_len(name: &str) -> usize {
    name.chars().flat_map(char::to_lowercase).map(char::len_utf8).sum::<usize>() + name.len() + 2
}

/// `files.order_key`: where a file sorts by name, all drives merged: its folder's key ([`folder_key`]), then its
/// name's. Made at its exact size.
pub fn order_key(folder_key: &[u8], name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(folder_key.len() + name_key_len(name));
    key.extend_from_slice(folder_key);
    push_name_key(&mut key, name);
    key
}

/// [`order_key`] of a file in the folder at `folder_path`.
pub fn order_key_of(folder_path: &str, name: &str) -> Vec<u8> {
    order_key(&folder_key(merged_path(folder_path)), name)
}

/// Whether `path` is `root` or inside it (Windows paths: case-insensitive).
pub fn is_under(path: &str, root: &str) -> bool {
    let (p, r) = (path.to_lowercase(), root.to_lowercase());
    p == r || p.strip_prefix(&r).is_some_and(|rest| rest.starts_with('\\'))
}

/// Paths kept end to end in one `String`, each named by its range ([`PathRef`]). It grows by doubling, so a list
/// of n paths makes about log₂ n allocations.
#[derive(Default, Debug)]
pub struct PathArena {
    text: String,
}

/// A path in a [`PathArena`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PathRef {
    start: usize,
    end: usize,
}

impl PathArena {
    /// `path` added at the end.
    pub fn push(&mut self, path: &str) -> PathRef {
        self.push_parts(&[path])
    }

    /// The concatenation of `parts`, added at the end.
    pub fn push_parts(&mut self, parts: &[&str]) -> PathRef {
        let start = self.text.len();
        for part in parts {
            self.text.push_str(part);
        }
        PathRef { start, end: self.text.len() }
    }

    pub fn get(&self, path: PathRef) -> &str {
        &self.text[path.start..path.end]
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
    pub items: Vec<T>,
    folders: PathArena,
    /// Each folder id's path in `folders`, once read.
    folder_paths: Vec<Option<PathRef>>,
    names: PathArena,
}

impl<T> WorkList<T> {
    /// Room for `items` items, and folder ids up to `max_folder_id`.
    pub fn with_capacity(items: usize, max_folder_id: i64) -> Self {
        let folder_paths = vec![None; usize::try_from(max_folder_id).map_or(0, |n| n + 1)];
        WorkList {
            items: Vec::with_capacity(items),
            folders: PathArena::default(),
            folder_paths,
            names: PathArena::default(),
        }
    }

    /// The path of the file `name` in folder `folder_id`. The folder's path is read by `read_folder` (into the
    /// arena it is given) the first time it is asked for.
    pub fn file<E>(
        &mut self,
        folder_id: i64,
        name: &str,
        read_folder: impl FnOnce(&mut PathArena) -> Result<PathRef, E>,
    ) -> Result<FilePath, E> {
        let k = usize::try_from(folder_id).unwrap_or(usize::MAX);
        let folder = match self.folder_paths.get(k).copied().flatten() {
            Some(folder) => folder,
            None => {
                let folder = read_folder(&mut self.folders)?;
                if k >= self.folder_paths.len() && k != usize::MAX {
                    // A folder made after the list was sized (not within one read).
                    self.folder_paths.resize(k + 1, None);
                }
                if let Some(slot) = self.folder_paths.get_mut(k) {
                    *slot = Some(folder);
                }
                folder
            }
        };
        Ok(FilePath { folder, name: self.names.push(name) })
    }

    /// The file's folder and name.
    pub fn parts(&self, path: FilePath) -> [&str; 2] {
        [self.folders.get(path.folder), self.names.get(path.name)]
    }

    /// Its path, for messages and for the programs it is handed to (FFmpeg).
    pub fn path(&self, path: FilePath) -> String {
        let [folder, name] = self.parts(path);
        [folder, "\\", name].concat()
    }

    /// Its path as [`verbatim_wide`] gives it, for opening it.
    pub fn wide(&self, path: FilePath, out: &mut Vec<u16>) {
        let [folder, name] = self.parts(path);
        verbatim_wide(&[folder, "\\", name], out);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> Default for WorkList<T> {
    fn default() -> Self {
        WorkList::with_capacity(0, -1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::SHORT;

    #[test]
    fn an_arena_gives_back_each_path() {
        let mut arena = PathArena::default();
        let refs: Vec<PathRef> = (0..1000).map(|i| arena.push(&format!(r"D:\folder {i}\file {i}.jpg"))).collect();
        let joined = arena.push_parts(&[r"E:\copies", r"\a.jpg"]);
        for (i, r) in refs.iter().enumerate() {
            assert_eq!(arena.get(*r), format!(r"D:\folder {i}\file {i}.jpg"));
        }
        assert_eq!(arena.get(joined), r"E:\copies\a.jpg");
        let empty = arena.push("");
        assert_eq!(arena.get(empty), "");
    }

    #[test]
    fn a_work_list_reads_each_folder_once() {
        let mut list = WorkList::<()>::with_capacity(4, 3);
        let mut reads = 0;
        let mut add = |list: &mut WorkList<()>, folder: i64, name: &str| {
            list.file(folder, name, |arena| {
                reads += 1;
                Ok::<_, ()>(arena.push(&format!(r"D:\f{folder}")))
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
            [a, b, c, d].map(|p| list.path(p)),
            [r"D:\f2\a.jpg", r"D:\f0\b.jpg", r"D:\f2\c.jpg", r"D:\f9\d.jpg"]
        );
        let mut wide = Vec::new();
        list.wide(c, &mut wide);
        assert_eq!(wide, r"D:\f2\c.jpg".encode_utf16().chain([0]).collect::<Vec<_>>());
        // A long one is verbatim, its name counted.
        let dir = "x".repeat(SHORT);
        let long = list.file(5, "e.jpg", |arena| Ok::<_, ()>(arena.push(&format!(r"D:\{dir}")))).unwrap();
        list.wide(long, &mut wide);
        assert_eq!(String::from_utf16(&wide[..wide.len() - 1]).unwrap(), format!(r"\\?\D:\{dir}\e.jpg"));
    }
}
