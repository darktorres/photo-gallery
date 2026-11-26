//! Paths under a set of rules: a path is an anchor (the text before the root: a Windows drive `D:`, the UNC
//! root `\`, or the empty anchor of a Unix path) and its parts, each a [`Name`]. The keys that order folders and
//! files, the containment test and the scope of a folder are functions of the parts under the rules. The drive is
//! not part of a folder's key, so one folder on every drive is one key (as the tree and the catalog have always had
//! it); the anchor is kept for display and compared in [`PathRules::is_under`].
//!
//! A name has three facets (`docs/plans/CATALOG_SIZE_PLAN.md`, D1): its identity is its exact bytes; its equivalence
//! (when two names are one name) is the file system's, which is the name key ([`PathRules::push_name_key`]: NTFS's
//! unit-by-unit upcase on Windows, the exact bytes on Posix); and the catalog's order is the byte order of the key, then
//! the row's id.
//!
//! [`PathRules`] is a closed enum, not a trait: two rule sets, both testable on every host, passed as a value.
//! Only [`PathRules::parse`] turns user text into a path; actions go by ids, not by path strings.

use alloc::string::String;
use alloc::vec::Vec;

use crate::name::Name;

/// The rules a path follows: how it is written, and how its names compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRules {
    /// Windows: `\` separates, a drive (`D:`) or the UNC root (`\\server\share`) is the anchor, names compare
    /// case-insensitively as NTFS does (each UTF-16 unit through the upcase table, `crate::upcase`) and folder keys
    /// are drive-free.
    Windows,
    /// POSIX: `/` separates, the anchor is always empty, and names are their exact bytes: compared and ordered as
    /// bytes (so `Pics` and `pics` are two names, and `Pics` sorts before `a`).
    Posix,
}

/// A path: its anchor (the text before the root separator) and its parts, in order. Serialises as
/// `{"anchor": name, "parts": [name, ...]}`, each a [`Name`] (its text, or its bytes when they are no UTF-8): a notice
/// that names a folder keeps it exact (`notice::Notice`).
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Path {
    /// What the path starts from: a drive, a share, or nothing.
    pub anchor: Name,
    /// The path's parts after the anchor, in order.
    pub parts: Vec<Name>,
}

impl Path {
    /// This path extended by one name.
    pub fn child(&self, name: &Name) -> Path {
        Path { anchor: self.anchor.clone(), parts: self.parts.iter().cloned().chain([name.clone()]).collect() }
    }

    /// The anchor's drive letter as a name (`D` for `D:\`); none for a UNC root or a Unix one, whose anchor has no
    /// letter once the `:` and the `\` are dropped.
    pub fn drive_letter(&self) -> Option<Name> {
        let letter: Vec<u8> = self.anchor.as_bytes().iter().copied().filter(|&b| b != b':' && b != b'\\').collect();
        (!letter.is_empty()).then(|| Name::from_raw(&letter))
    }
}

/// A folder as the tree and the catalog hold it: its parts, the anchor (the drive, or the UNC root) dropped.
/// The same folder on two drives is one `Merged`. The empty `Merged` is every folder: with `recursive` the whole
/// library, without it the files directly in the drive roots. Serialises as the array of its parts.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Merged(Vec<Name>);

impl Merged {
    /// The parts, from the top.
    pub fn parts(&self) -> &[Name] {
        &self.0
    }

    /// Whether it is the empty folder (every folder).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<Name>> for Merged {
    fn from(parts: Vec<Name>) -> Merged {
        Merged(parts)
    }
}

impl PathRules {
    /// The separator between parts.
    pub fn sep(self) -> char {
        match self {
            PathRules::Windows => '\\',
            PathRules::Posix => '/',
        }
    }

    /// The separator between parts, as its byte (both are ASCII).
    pub const fn sep_byte(self) -> u8 {
        match self {
            PathRules::Windows => b'\\',
            PathRules::Posix => b'/',
        }
    }

    /// The path that user text names, if it is one of these rules' paths. A Windows drive with or without its
    /// root (`D:`, `D:\a`), a UNC path (`\\server\share\a`, anchor `\`, parts `server`, `share`, `a`), or a
    /// relative path (empty anchor). Empty parts (doubled or trailing separators) and `.` parts are dropped, and on
    /// Windows `/` separates as `\` does (a Windows name holds neither, so a path typed or saved with forward slashes
    /// is the same path). `..` is kept: it is resolved where the path is taken from the disk. Returns `None` for text
    /// that cannot be a path here (a NUL, which no name holds).
    pub fn parse(self, text: &str) -> Option<Path> {
        self.parse_bytes(text.as_bytes())
    }

    /// [`PathRules::parse`] over the bytes a path is stored as ([`PathRules::join`]): canonical WTF-8 on Windows,
    /// raw bytes on Posix. The separators are ASCII, so splitting at them keeps every part canonical.
    pub fn parse_bytes(self, bytes: &[u8]) -> Option<Path> {
        if bytes.contains(&0) {
            return None;
        }
        // A Windows name holds no `/`, so a forward slash is a separator here too. Without this, a path with mixed
        // separators (`D:\/a/b`) parsed to one name `/a/b`, which joined back gave the same malformed text. The bytes
        // are ASCII, so no byte of a multi-byte character changes.
        let folded: Vec<u8>;
        let bytes = match self {
            PathRules::Windows if bytes.contains(&b'/') => {
                folded = bytes.iter().map(|&b| if b == b'/' { b'\\' } else { b }).collect();
                folded.as_slice()
            }
            _ => bytes,
        };
        let sep = self.sep_byte();
        let (anchor, rest): (&[u8], &[u8]) = match self {
            PathRules::Windows => {
                if let Some(rest) = bytes.strip_prefix(br"\\") {
                    (br"\", rest)
                } else if let Some((drive @ &[d, b':'], rest)) = bytes.split_at_checked(2) {
                    if d.is_ascii_alphabetic() { (drive, trim_seps(rest, sep)) } else { (b"", bytes) }
                } else {
                    (b"", bytes)
                }
            }
            PathRules::Posix => (b"", trim_seps(bytes, sep)),
        };
        let parts = rest.split(|&b| b == sep).filter(|p| !p.is_empty() && *p != b".").map(Name::from_raw).collect();
        Some(Path { anchor: Name::from_raw(anchor), parts })
    }

    /// A path's bytes, as the catalog stores them: the anchor, the root separator, then the parts joined by the
    /// separator. [`PathRules::parse_bytes`] reads them back.
    pub fn join(self, path: &Path) -> Vec<u8> {
        let mut out = Vec::with_capacity(path.anchor.as_bytes().len() + 1);
        out.extend_from_slice(path.anchor.as_bytes());
        out.push(self.sep_byte());
        self.push_parts(&mut out, &path.parts);
        out
    }

    /// The parts appended to `out`, joined by the separator (no anchor, no root separator).
    pub(crate) fn push_parts(self, out: &mut Vec<u8>, parts: &[Name]) {
        let sep = self.sep_byte();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            out.extend_from_slice(part.as_bytes());
        }
    }

    /// The folder a path names, its anchor dropped (see [`Merged`]).
    pub fn merge(self, path: &Path) -> Merged {
        Merged(path.parts.clone())
    }

    /// A folder's stored form: its parts joined by the separator, with no anchor. The empty folder is the empty
    /// bytes. [`PathRules::parse_merged`] reads them back.
    pub fn join_merged(self, folder: &Merged) -> Vec<u8> {
        let mut out = Vec::new();
        self.push_parts(&mut out, &folder.0);
        out
    }

    /// The folder a stored form ([`PathRules::join_merged`]) names. Empty parts (doubled separators) are dropped.
    /// `None` for a NUL, which no name holds.
    pub fn parse_merged(self, bytes: &[u8]) -> Option<Merged> {
        if bytes.contains(&0) {
            return None;
        }
        let sep = self.sep_byte();
        let parts = bytes.split(|&b| b == sep).filter(|p| !p.is_empty()).map(Name::from_raw).collect();
        Some(Merged(parts))
    }

    /// A folder's parts joined by the separator, for display only (the title, the HUD, messages): lossy.
    pub fn display_merged(self, folder: &Merged) -> String {
        let sep = self.sep();
        let mut out = String::new();
        for (i, part) in folder.0.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            out.push_str(&part.display());
        }
        out
    }

    /// The folder's key ([`PathRules::folder_key`] of its parts), as the tree and the catalog store it.
    pub fn merged_key(self, folder: &Merged) -> Vec<u8> {
        self.folder_key(&folder.0)
    }

    /// The path in UTF-16, as the Windows API takes it (no NUL): exact, unpaired surrogates included. On Posix it is
    /// the path's bytes widened, meaningless there, and not used.
    pub fn to_wide(self, path: &Path) -> Vec<u16> {
        Name::from_raw(&self.join(path)).to_wide()
    }

    /// A stored path (its bytes as [`PathRules::join`] wrote them) as text, as [`PathRules::display`] shows it; bytes that
    /// are no path (a NUL) show lossily as they are.
    pub fn display_stored(self, bytes: &[u8]) -> String {
        self.parse_bytes(bytes).map_or_else(|| String::from_utf8_lossy(bytes).into_owned(), |p| self.display(&p))
    }

    /// The path as text, lossy (for display and messages only): anchor, root separator, parts. A UNC path's anchor
    /// is its leading `\`, so the leading backslashes come back. A path is rooted, so the separator is always
    /// there (a POSIX root is `/`).
    pub fn display(self, path: &Path) -> String {
        let sep = self.sep();
        let mut out = path.anchor.display().into_owned();
        out.push(sep);
        for (i, part) in path.parts.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            out.push_str(&part.display());
        }
        out
    }

    /// A name's key under these rules, appended to `out`: the name in the form two names that are one name share, and
    /// whose byte order is the catalog's name order. Windows: each UTF-16 unit through the embedded upcase table, as
    /// WTF-8 ([`Name::push_upcase`]): NTFS's own comparison, the same on every machine. Posix: the exact bytes. The
    /// catalog's `path_name_key` SQL function and its expression index are this function.
    pub fn push_name_key(self, name: &Name, out: &mut Vec<u8>) {
        match self {
            PathRules::Windows => name.push_upcase(out),
            PathRules::Posix => out.extend_from_slice(name.as_bytes()),
        }
    }

    /// [`PathRules::push_name_key`] as a new vector.
    pub fn name_key(self, name: &Name) -> Vec<u8> {
        let mut key = Vec::with_capacity(name.as_bytes().len());
        self.push_name_key(name, &mut key);
        key
    }

    /// The folder's key (the drive-free key that orders folders: the catalog's `mfolders.key`), appended to `key`: each
    /// part's name key ([`PathRules::push_name_key`]), then 0x01. A byte 0x01 or 0x02 of a part's key is escaped as
    /// 0x02 0x02 or 0x02 0x03, which keeps the order, so parts are prefix-free under the terminator and a folder's key
    /// is a prefix of its subfolders' keys.
    pub fn push_folder_key(self, key: &mut Vec<u8>, parts: &[Name]) {
        let mut part_key = Vec::new();
        for part in parts {
            part_key.clear();
            self.push_name_key(part, &mut part_key);
            push_escaped(key, &part_key);
            key.push(0x01);
        }
    }

    /// [`PathRules::push_folder_key`] as a new vector.
    pub fn folder_key(self, parts: &[Name]) -> Vec<u8> {
        let mut key = Vec::new();
        self.push_folder_key(&mut key, parts);
        key
    }

    /// The folder keys a folder's scope covers, as a range `lo..hi` (`hi` is `None` when nothing sorts above it): the
    /// folder's own key alone, or with `recursive` its key and every key below it. A file is in the scope when its
    /// folder's key is.
    pub fn scope_keys(self, parts: &[Name], recursive: bool) -> (Vec<u8>, Option<Vec<u8>>) {
        let lo = self.folder_key(parts);
        let hi = if recursive {
            // Every key below the folder starts with its key, and nothing else does from `lo` up to the bound.
            prefix_upper_bound(&lo)
        } else {
            // The least key above `lo` is `lo` 0x00, and a subfolder's key is `lo`, then an escaped byte (never below
            // 0x02, since no name holds a NUL and 0x01 is escaped), so the range holds `lo` alone.
            Some([lo.as_slice(), &[0x00]].concat())
        };
        (lo, hi)
    }

    /// The key that finds a folder by its spelling under these rules, unique to that spelling: the anchor (a Windows
    /// drive or UNC root as its name key, then 0x00; a POSIX path has none), then the folder key of its parts
    /// ([`PathRules::push_folder_key`]). Each byte of the anchor is escaped as the parts are, so the terminators stay
    /// unambiguous. A Windows part is compared by its upcase key, a POSIX part exactly, as [`PathRules::is_under`]
    /// compares them. A folder's key is a prefix of its subfolders' keys, so the subtree of a folder is the range
    /// [`PathRules::subtree`] gives.
    ///
    /// Worked examples: `C:\Photos\Raw` is `C:` 0x00 `PHOTOS` 0x01 `RAW` 0x01, and `/photos/raw` is `photos` 0x01
    /// `raw` 0x01 (the POSIX root is the empty key).
    ///
    /// Not folded, so one folder can still be two keys: the drive letter (a re-lettered drive is another anchor, as
    /// [`PathRules::parse`] keeps it), a verbatim volume spelling (`\\?\Volume{...}`), a volume whose own `$UpCase`
    /// differs from the embedded table, trailing dots and spaces, 8.3 short names, and junctions.
    pub fn path_key(self, path: &Path) -> Vec<u8> {
        let mut key = Vec::new();
        if self == PathRules::Windows {
            push_escaped(&mut key, &self.name_key(&path.anchor));
            key.push(0x00);
        }
        self.push_folder_key(&mut key, &path.parts);
        key
    }

    /// The range of the keys in the subtree of `path`, as `(key, upper)`: a key is in the subtree (the folder itself and
    /// every folder below it) exactly when it is at least `key` and below `upper`. `upper` is `None` when no key lies
    /// above the subtree, which is the POSIX root's (its key is empty, and every key is in its subtree).
    pub fn subtree(self, path: &Path) -> (Vec<u8>, Option<Vec<u8>>) {
        let key = self.path_key(path);
        let upper = prefix_upper_bound(&key);
        (key, upper)
    }

    /// Whether `path` is `root` or inside it. The anchors must agree (case-insensitively on Windows), then the
    /// parts must start with the root's parts (compared by the same case rule as the keys).
    pub fn is_under(self, path: &Path, root: &Path) -> bool {
        self.same_anchor(&path.anchor, &root.anchor)
            && root.parts.len() <= path.parts.len()
            && path.parts.iter().zip(&root.parts).all(|(a, b)| self.same_part(a, b))
    }

    /// Whether two paths are the same path under these rules: the same anchor and the same parts (compared as
    /// [`PathRules::is_under`] compares them, so a Windows path is the same in any case).
    pub fn same(self, a: &Path, b: &Path) -> bool {
        self.is_under(a, b) && self.is_under(b, a)
    }

    fn same_anchor(self, a: &Name, b: &Name) -> bool {
        match self {
            PathRules::Windows => self.same_part(a, b),
            PathRules::Posix => a == b,
        }
    }

    fn same_part(self, a: &Name, b: &Name) -> bool {
        match self {
            PathRules::Windows => self.name_key(a) == self.name_key(b),
            PathRules::Posix => a == b,
        }
    }

    /// Splits a file's path into its folder and its name. `None` for a path with no parts (an anchor alone).
    pub fn split_file_name(self, path: &Path) -> Option<(Path, Name)> {
        let (name, folder) = path.parts.split_last()?;
        Some((Path { anchor: path.anchor.clone(), parts: folder.to_vec() }, name.clone()))
    }
}

/// The least key above every key that starts with `prefix` (a folder's key), so `prefix..bound` is exactly the keys
/// that start with it: `None` when there is none (the empty folder, whose prefix every key starts with). A folder key
/// ends in its part terminator 0x01, so the bound is `prefix[..-1] + [0x02]`, and it is exact: a key that starts with
/// the prefix is below it whatever bytes follow (a subfolder whose raw Posix name starts with byte 0xFF included),
/// and a key from the prefix up to it starts with the prefix. The rule in general, which gives that: the trailing 0xFF
/// bytes go (nothing is above them), and the last byte left goes up by one.
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut hi = prefix.to_vec();
    while let Some(last) = hi.pop() {
        if let Some(next) = last.checked_add(1) {
            hi.push(next);
            return Some(hi);
        }
    }
    None
}

/// `bytes` appended with the key's escapes: 0x01 as 0x02 0x02 and 0x02 as 0x02 0x03, so the terminators stay
/// unambiguous and the order is kept.
fn push_escaped(key: &mut Vec<u8>, bytes: &[u8]) {
    for &b in bytes {
        match b {
            0x01 => key.extend([0x02, 0x02]),
            0x02 => key.extend([0x02, 0x03]),
            _ => key.push(b),
        }
    }
}

/// `bytes` without its leading separators.
fn trim_seps(bytes: &[u8], sep: u8) -> &[u8] {
    let n = bytes.iter().take_while(|&&b| b == sep).count();
    bytes.get(n..).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn names(parts: &[&str]) -> Vec<Name> {
        parts.iter().map(|p| Name::from(*p)).collect()
    }

    #[test]
    fn a_windows_path_splits_into_its_anchor_and_parts() {
        let p = PathRules::Windows.parse(r"D:\pinterest\\a\").unwrap();
        assert_eq!(p.anchor, Name::from("D:"));
        assert_eq!(p.parts, names(&["pinterest", "a"]));
        assert_eq!(PathRules::Windows.display(&p), r"D:\pinterest\a");
        let unc = PathRules::Windows.parse(r"\\server\share\x").unwrap();
        assert_eq!(unc.anchor, Name::from(r"\"));
        assert_eq!(unc.parts, names(&["server", "share", "x"]));
        assert_eq!(PathRules::Windows.display(&unc), r"\\server\share\x");
        assert_eq!(PathRules::Windows.display(&PathRules::Windows.parse("D:\\").unwrap()), "D:\\");
    }

    /// Forward slashes are separators on Windows, as backslashes are; `.` parts go. The saved value that made the
    /// picker's dialog fail reads back as the folder it names, and joins back in the form the shell takes.
    #[test]
    fn windows_text_takes_forward_slashes_and_drops_dot_parts() {
        let back = PathRules::Windows.parse(r"D:\PhotoGallery\datasets\pinterest\Photos").unwrap();
        assert_eq!(PathRules::Windows.parse("D:/PhotoGallery/datasets/pinterest/Photos").unwrap(), back);
        assert_eq!(PathRules::Windows.parse(r"D:\/PhotoGallery/datasets/pinterest/Photos").unwrap(), back);
        assert_eq!(PathRules::Windows.parse(r"D:\a\.\b").unwrap(), PathRules::Windows.parse(r"D:\a\b").unwrap());
        assert_eq!(
            PathRules::Windows.parse("//server/share/a").unwrap(),
            PathRules::Windows.parse(r"\\server\share\a").unwrap()
        );
        let saved = PathRules::Windows.parse_bytes(br"D:\/PhotoGallery/datasets/pinterest/Photos").unwrap();
        assert_eq!(PathRules::Windows.join(&saved), br"D:\PhotoGallery\datasets\pinterest\Photos");
    }

    #[test]
    fn a_posix_path_has_an_empty_anchor() {
        let p = PathRules::Posix.parse("/home//a/").unwrap();
        assert_eq!(p.anchor, Name::from(""));
        assert_eq!(p.parts, names(&["home", "a"]));
        assert_eq!(PathRules::Posix.display(&p), "/home/a");
        assert_eq!(PathRules::Posix.display(&PathRules::Posix.parse("/").unwrap()), "/");
    }

    #[test]
    fn the_drive_is_not_part_of_a_folder_key() {
        let a = PathRules::Windows.parse(r"E:\pinterest\a").unwrap();
        let b = PathRules::Windows.parse(r"D:\PINTEREST\A").unwrap();
        assert_eq!(PathRules::Windows.folder_key(&a.parts), PathRules::Windows.folder_key(&b.parts));
    }

    #[test]
    fn posix_names_differing_in_case_are_two_folders_in_byte_order() {
        let lower = PathRules::Posix.folder_key(&names(&["pics"]));
        let upper = PathRules::Posix.folder_key(&names(&["Pics"]));
        assert_ne!(lower, upper);
        assert!(upper < PathRules::Posix.folder_key(&names(&["a"])), "Pics sorts before a: byte order");
        assert_ne!(PathRules::Posix.folder_key(&names(&["pics"])), PathRules::Posix.folder_key(&names(&["Pics", "x"])),);
    }

    #[test]
    fn a_name_key_is_the_upcase_on_windows_and_the_bytes_on_posix() {
        let name = Name::from("Ασ.jpg");
        assert_eq!(PathRules::Windows.name_key(&name), "ΑΣ.JPG".as_bytes());
        assert_eq!(PathRules::Posix.name_key(&name), name.as_bytes());
        assert_eq!(PathRules::Windows.name_key(&Name::from("αΣ.JPG")), PathRules::Windows.name_key(&name));
    }

    #[test]
    fn containment_compares_anchors_and_parts() {
        let root = PathRules::Windows.parse(r"D:\a").unwrap();
        let inside = PathRules::Windows.parse(r"d:\A\b").unwrap();
        let other_drive = PathRules::Windows.parse(r"E:\a\b").unwrap();
        let sibling = PathRules::Windows.parse(r"D:\ab").unwrap();
        assert!(PathRules::Windows.is_under(&inside, &root));
        assert!(PathRules::Windows.is_under(&root, &root));
        assert!(!PathRules::Windows.is_under(&other_drive, &root));
        assert!(!PathRules::Windows.is_under(&sibling, &root));
    }

    #[test]
    fn a_file_splits_from_its_folder() {
        let file = PathRules::Windows.parse(r"D:\a\b.jpg").unwrap();
        let (folder, name) = PathRules::Windows.split_file_name(&file).unwrap();
        assert_eq!(PathRules::Windows.display(&folder), r"D:\a");
        assert_eq!(name, Name::from("b.jpg"));
    }

    /// The folder key of the file at `text` (a Windows path): what a scope range is tested against.
    fn file_key(text: &str) -> Vec<u8> {
        let path = PathRules::Windows.parse(text).unwrap();
        let (folder, _) = PathRules::Windows.split_file_name(&path).unwrap();
        PathRules::Windows.folder_key(&folder.parts)
    }

    #[test]
    fn a_folder_scope_is_its_own_files_or_those_and_its_subfolders() {
        let within = |key: &[u8], (lo, hi): &(Vec<u8>, Option<Vec<u8>>)| {
            key >= lo.as_slice() && hi.as_ref().is_none_or(|hi| key < hi.as_slice())
        };
        let direct = file_key(r"E:\pinterest\a.jpg");
        let sub = file_key(r"D:\pinterest\sub\a.jpg");
        let sibling = file_key(r"D:\pinterest-x\a.jpg");
        let own = PathRules::Windows.scope_keys(&names(&["pinterest"]), false);
        assert!(within(&direct, &own) && !within(&sub, &own) && !within(&sibling, &own));
        let tree = PathRules::Windows.scope_keys(&names(&["pinterest"]), true);
        assert!(within(&direct, &tree) && within(&sub, &tree) && !within(&sibling, &tree));
    }

    #[test]
    fn a_sigma_folder_scopes_its_files_in_any_case() {
        // σ upcases to Σ, so ασ and ΑΣ are one folder key, and one scope; the final ς keeps its own name (as on NTFS),
        // so ας is another folder.
        let file = file_key(r"D:\ασ\a.jpg");
        let (lo, hi) = PathRules::Windows.scope_keys(&names(&["ΑΣ"]), false);
        assert!(file >= lo && hi.as_ref().is_some_and(|hi| file < *hi));
        let final_sigma = file_key(r"D:\ας\a.jpg");
        assert!(!(final_sigma >= lo && hi.is_some_and(|hi| final_sigma < hi)));
    }

    #[test]
    fn the_empty_merged_folder_is_every_folder_with_recursive_and_the_drive_roots_without() {
        let empty = Merged::default();
        assert!(empty.is_empty());
        let (lo, hi) = PathRules::Windows.scope_keys(empty.parts(), true);
        assert!(lo.is_empty() && hi.is_none());
        let (lo, hi) = PathRules::Windows.scope_keys(empty.parts(), false);
        let root_file = file_key(r"D:\a.jpg");
        let sub_file = file_key(r"D:\a\b.jpg");
        assert!(root_file >= lo && root_file < hi.clone().unwrap() && sub_file >= hi.unwrap());
    }

    #[test]
    fn a_merged_folder_is_its_parts_without_the_drive_and_joins_back_exactly() {
        let path = PathRules::Windows.parse(r"E:\pinterest\a").unwrap();
        let merged = PathRules::Windows.merge(&path);
        assert_eq!(merged.parts(), names(&["pinterest", "a"]).as_slice());
        let stored = PathRules::Windows.join_merged(&merged);
        assert_eq!(stored, br"pinterest\a");
        assert_eq!(PathRules::Windows.parse_merged(&stored), Some(merged.clone()));
        assert_eq!(PathRules::Windows.display_merged(&merged), r"pinterest\a");
        assert_eq!(PathRules::Windows.merged_key(&merged), PathRules::Windows.folder_key(&names(&["pinterest", "a"])));
        assert_eq!(PathRules::Windows.join_merged(&Merged::default()), b"");
        assert_eq!(PathRules::Windows.parse_merged(b""), Some(Merged::default()));
        assert_eq!(PathRules::Windows.parse_merged(b"a\\\\b\\"), Some(Merged::from(names(&["a", "b"]))));
        assert_eq!(PathRules::Windows.parse_merged(b"a\0b"), None);
    }

    #[test]
    fn a_merged_folder_keeps_a_lone_surrogate_and_a_raw_byte_exactly() {
        let wide = Name::from_wide(&[0x61, 0xD800]);
        let raw = Name::from_raw(b"x\xFF");
        let merged = Merged::from(vec![wide.clone(), raw.clone()]);
        let stored = PathRules::Posix.join_merged(&merged);
        assert_eq!(stored, [wide.as_bytes(), &b"/"[..], raw.as_bytes()].concat());
        assert_eq!(PathRules::Posix.parse_merged(&stored), Some(merged));
    }

    #[test]
    fn a_merged_folder_round_trips_through_json_as_the_array_of_its_parts() {
        let merged = Merged::from(vec![Name::from("pics"), Name::from_wide(&[0xD800])]);
        let json = serde_json::to_string(&merged).unwrap();
        assert_eq!(json, r#"["pics",{"b":"eda080"}]"#);
        assert_eq!(serde_json::from_str::<Merged>(&json).unwrap(), merged);
        assert_eq!(serde_json::to_string(&Merged::default()).unwrap(), "[]");
    }

    #[test]
    fn a_prefix_bound_is_the_least_key_above_the_prefix() {
        assert_eq!(prefix_upper_bound(&[]), None);
        assert_eq!(prefix_upper_bound(&[0x01]), Some(vec![0x02]));
        assert_eq!(prefix_upper_bound(&[0xFF, 0x01]), Some(vec![0xFF, 0x02]));
        // The general rule (not a folder key's): trailing 0xFF bytes go first.
        assert_eq!(prefix_upper_bound(&[0x61, 0xFF]), Some(vec![0x62]));
        assert_eq!(prefix_upper_bound(&[0xFF, 0xFF]), None);
    }

    #[test]
    fn a_recursive_scope_holds_a_posix_subfolder_whose_name_starts_with_0xff() {
        let rules = PathRules::Posix;
        let parent = names(&["photos"]);
        let (lo, hi) = rules.scope_keys(&parent, true);
        let hi = hi.unwrap();
        let mut below = parent.clone();
        below.push(Name::from_raw(b"\xFFa"));
        let key = rules.folder_key(&below);
        assert!(lo <= key && key < hi, "{key:?} not in {lo:?}..{hi:?}");
        // A sibling whose name extends the folder's is outside.
        let sibling = rules.folder_key(&names(&["photos\u{1}"]));
        assert!(sibling >= hi);
        // Without `recursive`, the folder's key alone.
        let (own, own_hi) = rules.scope_keys(&parent, false);
        let own_hi = own_hi.unwrap();
        assert!(own <= rules.folder_key(&parent) && rules.folder_key(&parent) < own_hi);
        assert!(key >= own_hi, "a subfolder is not in the folder's own scope");
    }
}

#[cfg(test)]
mod byte_tests {
    use alloc::format;
    use alloc::vec;

    use super::*;

    #[test]
    fn a_path_joins_and_parses_back_exactly_even_with_a_lone_surrogate() {
        let wide: Vec<u16> = r"D:\a\".encode_utf16().chain([0xD800]).chain(r"\b.jpg".encode_utf16()).collect();
        let path = Path {
            anchor: Name::from("D:"),
            parts: vec![Name::from("a"), Name::from_wide(&wide[5..6]), Name::from("b.jpg")],
        };
        let bytes = PathRules::Windows.join(&path);
        assert_eq!(
            bytes,
            br"D:\a\".iter().copied().chain([0xED, 0xA0, 0x80]).chain(br"\b.jpg".iter().copied()).collect::<Vec<u8>>()
        );
        assert_eq!(PathRules::Windows.parse_bytes(&bytes), Some(path.clone()));
        assert_eq!(PathRules::Windows.to_wide(&path), wide);
    }

    #[test]
    fn a_posix_path_keeps_raw_bytes() {
        let path = PathRules::Posix.parse_bytes(b"/home/\xFF/x").unwrap();
        assert_eq!(PathRules::Posix.join(&path), b"/home/\xFF/x");
    }

    fn key_of(rules: PathRules, text: &str) -> Vec<u8> {
        rules.path_key(&rules.parse(text).unwrap())
    }

    // The worked examples of `path_key`: a Windows folder is its drive, then its upcased parts; a POSIX folder is its
    // exact parts, and the root is the empty key.
    #[test]
    fn a_folder_key_is_its_anchor_then_its_parts() {
        assert_eq!(key_of(PathRules::Windows, r"C:\Photos\Raw"), b"C:\x00PHOTOS\x01RAW\x01");
        assert_eq!(key_of(PathRules::Windows, r"c:\photos\raw"), b"C:\x00PHOTOS\x01RAW\x01");
        assert_eq!(key_of(PathRules::Posix, "/photos/raw"), b"photos\x01raw\x01");
        assert_eq!(key_of(PathRules::Posix, "/"), b"");
        assert_eq!(key_of(PathRules::Windows, r"D:\"), b"D:\x00");
    }

    // The spelling is the key: a Windows folder in another case is the same key, a POSIX one is not.
    #[test]
    fn the_same_folder_in_another_case_has_the_same_windows_key_and_a_posix_one_has_not() {
        assert_eq!(key_of(PathRules::Windows, r"D:\Photos"), key_of(PathRules::Windows, r"d:\PHOTOS"));
        assert_ne!(key_of(PathRules::Posix, "/Photos"), key_of(PathRules::Posix, "/photos"));
    }

    // The edge cases of the Windows fold, pinned: the key upcases each UTF-16 unit by the embedded table, NTFS's way, so
    // a letter is one name with its single-unit uppercase when that lowercases back to it, and with nothing else (gate
    // G2 of the catalog-size plan checked the table against NTFS itself, over the whole BMP).
    #[test]
    fn the_windows_fold_is_the_tables_unit_by_unit() {
        let key = |text: &str| key_of(PathRules::Windows, &format!(r"D:\{text}"));
        // One unit upcases to the other, and back: one folder.
        for (a, b) in [("Σ", "σ"), ("é", "É"), ("ÿ", "Ÿ")] {
            assert_eq!(key(a), key(b), "{a} and {b}");
        }
        // Two folders: an uppercase that lowercases to another letter (ς, ſ and ı against Σ, S and I), the Kelvin sign
        // and İ (their own upper case), ß (none in one unit), and a supplementary letter (Deseret: not folded at all).
        for (a, b) in
            [("σ", "ς"), ("ſ", "s"), ("ı", "i"), ("İ", "i"), ("\u{212A}", "k"), ("ß", "ẞ"), ("\u{10428}", "\u{10400}")]
        {
            assert_ne!(key(a), key(b), "{a} and {b}");
        }
    }

    // A part's terminator is not a byte of a name: a folder named `b` followed by a byte 0x01 is not under `b`, though
    // its key begins with `b`'s.
    #[test]
    fn a_folder_is_not_a_prefix_of_a_sibling_whose_name_extends_it() {
        let (key, upper) = PathRules::Posix.subtree(&PathRules::Posix.parse("/a/b").unwrap());
        let sibling = Path { anchor: Name::from(""), parts: vec![Name::from("a"), Name::from_raw(b"b\x01c")] };
        let child = PathRules::Posix.path_key(&sibling);
        assert!(
            !(child >= key && child < upper.unwrap()),
            "a folder named `b` with a terminator byte is not under `b`"
        );
    }

    // A relative path whose one part is `c:x` has no drive: its key is not in the subtree of the drive `C:`.
    #[test]
    fn a_relative_part_named_like_a_drive_is_not_in_the_drive() {
        let relative = Path { anchor: Name::from(""), parts: vec![Name::from("c:x")] };
        let (key, upper) = PathRules::Windows.subtree(&PathRules::Windows.parse(r"C:\").unwrap());
        let relative_key = PathRules::Windows.path_key(&relative);
        assert!(!(relative_key >= key && relative_key < upper.unwrap()), "{relative_key:?}");
    }

    // The range of a subtree holds exactly the folders the oracle `is_under` names, over spellings that vary in case,
    // separators and prefixes, for both rule sets.
    #[test]
    fn a_subtree_range_holds_exactly_the_folders_under_its_root() {
        let windows = [
            r"C:\",
            r"C:\a",
            r"C:\A\b",
            r"C:\a\b",
            r"C:\ab",
            r"C:\a.b",
            r"C:\a\b\c",
            r"C:\a\b\c\d",
            r"C:\b",
            r"D:\a",
            r"C:\a\b\",
            r"C:\a\b\c\",
        ];
        let posix = ["/", "/a", "/a/b", "/a/bc", "/a/b/c", "/a\u{1}b", "/A/B", "/a/b\u{1}c", "/a/b\u{2}"];
        for (rules, texts) in [(PathRules::Windows, windows.as_slice()), (PathRules::Posix, posix.as_slice())] {
            let paths: Vec<Path> = texts.iter().filter_map(|t| rules.parse(t)).collect();
            for root in &paths {
                let (key, upper) = rules.subtree(root);
                for path in &paths {
                    let k = rules.path_key(path);
                    let inside = k >= key && upper.as_ref().is_none_or(|u| k < *u);
                    assert_eq!(inside, rules.is_under(path, root), "{rules:?}: {path:?} under {root:?}");
                }
            }
        }
    }
}
