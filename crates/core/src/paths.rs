//! Windows paths handed outside the standard library. `std::fs` already turns a long path into its verbatim form
//! (`\\?\`) itself, so a path for std is used as it is. FFmpeg and direct Win32 calls (the scanner's
//! `CreateFileW`) don't: a path past the 260-character limit reaches them in its verbatim form, made here.
//! ExifTool takes the plain path, as a `\\?\` path isn't one it reads (its own long-name support is a separate
//! option, FINDINGS "I/O").
//!
//! A pass's paths are kept end to end in one string ([`PathArena`], [`WorkList`]): a list of 189k files is a few
//! allocations, not one per path.

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

/// [`verbatim`] of `path` followed by `tail` (which counts towards the length), as NUL-terminated UTF-16 for a
/// Win32 call, written into `out`.
pub fn verbatim_wide(path: &str, tail: &str, out: &mut Vec<u16>) {
    let (prefix, rest) = split(path, path.len() + tail.len());
    out.clear();
    out.extend(prefix.encode_utf16().chain(rest.encode_utf16()).chain(tail.encode_utf16()).chain([0]));
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

/// A pass's work: its items, each holding its path as a [`PathRef`] into `paths`.
pub struct WorkList<T> {
    pub items: Vec<T>,
    pub paths: PathArena,
}

impl<T> WorkList<T> {
    pub fn path(&self, path: PathRef) -> &str {
        self.paths.get(path)
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
        WorkList { items: Vec::new(), paths: PathArena::default() }
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
            verbatim_wide(path, tail, &mut wide);
            let whole = verbatim(&format!("{path}{tail}")).encode_utf16().chain([0]).collect::<Vec<_>>();
            assert_eq!(wide, whole, "{path}");
        }
        // The tail counts: a folder just short of the limit is long with its backslash.
        let edge = format!(r"D:\{}", "x".repeat(SHORT - 4));
        assert_eq!(edge.len(), SHORT - 1);
        verbatim_wide(&edge, r"\", &mut wide);
        assert!(String::from_utf16(&wide).unwrap().starts_with(r"\\?\D:"));
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
