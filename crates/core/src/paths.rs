//! Windows paths handed outside the standard library. `std::fs` already turns a long path into its verbatim form
//! (`\\?\`) itself, so a path for std is used as it is. FFmpeg and direct Win32 calls (the scanner's
//! `CreateFileW`) don't: a path past the 260-character limit reaches them in its verbatim form, made here.
//! ExifTool takes the plain path, as a `\\?\` path isn't one it reads (its own long-name support is a separate
//! option, FINDINGS "I/O").
//!
//! A pass's paths are kept end to end in strings ([`PathArena`]): a [`WorkList`] holds each folder's path once and
//! each file's name, so a list of 189k files is a few allocations, not one per path. A file is opened from them
//! through a reader's wide buffer ([`WorkList::wide`]), without a path of its own.

use std::borrow::Cow;

/// Shorter than this, a path (a folder, or a file in one) is used as it is.
const SHORT: usize = 240;

/// The verbatim form of an absolute path (`D:\…` or `\\server\share\…`) when it is long, else the path itself.
pub fn verbatim(path: &str) -> Cow<'_, str> {
    match split(path, path.len()) {
        ("", path) => Cow::Borrowed(path),
        (prefix, rest) => Cow::Owned([prefix, rest].concat()),
    }
}

/// [`verbatim`] of the concatenation of `parts` (the first starts the path), as NUL-terminated UTF-16 for a Win32
/// call, written into `out`.
pub fn verbatim_wide(parts: &[&str], out: &mut Vec<u16>) {
    let (first, tail) = parts.split_first().map_or(("", &[][..]), |(f, t)| (*f, t));
    let (prefix, rest) = split(first, parts.iter().map(|p| p.len()).sum());
    out.clear();
    out.extend(prefix.encode_utf16().chain(rest.encode_utf16()));
    for part in tail {
        out.extend(part.encode_utf16());
    }
    out.push(0);
}

/// What the verbatim form of a path `len` bytes long that starts with `path` puts first, and the part of `path`
/// that follows it (the leading `\\` of a share gives way to `\\?\UNC\`).
fn split(path: &str, len: usize) -> (&'static str, &str) {
    if len < SHORT || path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        ("", path)
    } else if let Some(unc) = path.strip_prefix(r"\\") {
        (r"\\?\UNC\", unc)
    } else {
        (r"\\?\", path)
    }
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
    fn long_paths_are_verbatim_unc_too() {
        assert_eq!(verbatim(r"D:\a\b.jpg"), r"D:\a\b.jpg");
        let dir = "x".repeat(250);
        assert_eq!(verbatim(&format!(r"D:\{dir}\b.jpg")), format!(r"\\?\D:\{dir}\b.jpg"));
        assert_eq!(verbatim(&format!(r"\\nas\photos\{dir}\b.jpg")), format!(r"\\?\UNC\nas\photos\{dir}\b.jpg"));
        let already = format!(r"\\?\D:\{dir}");
        assert_eq!(verbatim(&already), already);
        let mut wide = Vec::new();
        for (path, tail) in [(r"D:\a", r"\"), (&*format!(r"D:\{dir}"), r"\"), (&*format!(r"\\nas\{dir}"), r"\")] {
            verbatim_wide(&[path, tail], &mut wide);
            let whole = verbatim(&format!("{path}{tail}")).encode_utf16().chain([0]).collect::<Vec<_>>();
            assert_eq!(wide, whole, "{path}");
        }
        // The tail counts: a folder just short of the limit is long with its backslash.
        let edge = format!(r"D:\{}", "x".repeat(SHORT - 4));
        assert_eq!(edge.len(), SHORT - 1);
        verbatim_wide(&[&edge, r"\"], &mut wide);
        assert!(String::from_utf16(&wide).unwrap().starts_with(r"\\?\D:"));
    }

    /// A work list gives each file's path from its folder, read once, and its name.
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

    /// std reads a file through a path past the limit as it is (it makes the verbatim form itself).
    #[test]
    fn std_takes_long_paths_as_they_are() {
        let root = std::env::temp_dir().join(format!("pg-paths-{}", std::process::id()));
        let mut dir = root.clone();
        while dir.as_os_str().len() < 300 {
            dir.push("a-folder-name-of-forty-characters-long--");
        }
        let file = dir.join("f.bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, b"long").unwrap();
        assert!(file.as_os_str().len() > 300);
        assert_eq!(std::fs::read(&file).unwrap(), b"long");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
