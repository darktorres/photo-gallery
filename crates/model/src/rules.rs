//! Paths under a set of rules (S-17.2): a path is an anchor (the text before the root: a Windows drive `D:`, the UNC
//! root `\`, or the empty anchor of a Unix path) and its parts, each a [`Name`]. The keys that order folders and
//! files, the containment test and the scope of a folder are functions of the parts under the rules. The drive is
//! not part of a folder's key, so one folder on every drive is one key (as the tree and the catalog have always had
//! it); the anchor is kept for display and compared in [`PathRules::is_under`].
//!
//! [`PathRules`] is a closed enum, not a trait: two rule sets, both testable on every host, passed as a value.
//! Only [`PathRules::parse`] turns user text into a path; actions go by ids, not by path strings.

use crate::name::Name;

/// The rules a path follows: how it is written, and how its names compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRules {
    /// Windows: `\` separates, a drive (`D:`) or the UNC root (`\\server\share`) is the anchor, names compare
    /// case-insensitively (per character, Unicode) and folder keys are drive-free.
    Windows,
    /// POSIX: `/` separates, the anchor is always empty, names are case-sensitive but ordered case-insensitively
    /// first (so `Pics` and `pics` are distinct, and sort together).
    Posix,
}

/// A path: its anchor (the text before the root separator) and its parts, in order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Path {
    pub anchor: Name,
    pub parts: Vec<Name>,
}

/// A folder as the tree and the catalog hold it (S-17.8): its parts, the anchor (the drive, or the UNC root) dropped.
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
    /// The rules of this platform.
    pub const NATIVE: PathRules = if cfg!(windows) { PathRules::Windows } else { PathRules::Posix };

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

    /// A folder's parts joined by the separator, for display only (the title, the Hud, messages): lossy.
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

    /// The folder's key (the part of an order key that orders folders), appended to `key`. Windows: each part in
    /// per-character lower case, then 0x01 (a byte 0x01 or 0x02 in the lower-cased name is escaped as 0x02 0x02
    /// or 0x02 0x03, which keeps the order). Posix: each part as its lower case, 0x00, its exact bytes, 0x01, both
    /// escaped the same way. Parts are prefix-free under the terminator, so a folder's key is a prefix of its
    /// subfolders' keys.
    pub fn push_folder_key(self, key: &mut Vec<u8>, parts: &[Name]) {
        for part in parts {
            match self {
                PathRules::Windows => {
                    let mut lower = Vec::with_capacity(part.as_bytes().len());
                    part.push_lower(&mut lower);
                    push_escaped(key, &lower);
                    key.push(0x01);
                }
                PathRules::Posix => {
                    let mut lower = Vec::with_capacity(part.as_bytes().len());
                    part.push_lower(&mut lower);
                    push_escaped(key, &lower);
                    key.push(0x00);
                    push_escaped(key, part.as_bytes());
                    key.push(0x01);
                }
            }
        }
    }

    /// [`PathRules::push_folder_key`] as a new vector.
    pub fn folder_key(self, parts: &[Name]) -> Vec<u8> {
        let mut key = Vec::new();
        self.push_folder_key(&mut key, parts);
        key
    }

    /// The order key of a file: its folder's key, then its name's ([`Name::push_name_key`]).
    pub fn order_key(self, folder_key: &[u8], name: &Name) -> Vec<u8> {
        let mut key = Vec::with_capacity(folder_key.len() + name.as_bytes().len() * 2 + 2);
        key.extend_from_slice(folder_key);
        name.push_name_key(&mut key);
        key
    }

    /// The order keys of a folder's files as a range `lo..hi` (`hi` is `None` when nothing sorts above it): the
    /// folder's own files, or with `recursive` every file below it too.
    pub fn scope_keys(self, parts: &[Name], recursive: bool) -> (Vec<u8>, Option<Vec<u8>>) {
        let mut lo = self.folder_key(parts);
        let hi = if recursive {
            // Every key below the folder starts with its key, and nothing else does from `lo` up to the bound.
            prefix_upper_bound(&lo)
        } else {
            let hi = [lo.as_slice(), &[0x01]].concat();
            lo.push(0x00);
            Some(hi)
        };
        (lo, hi)
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
            PathRules::Windows => {
                let (mut la, mut lb) = (Vec::new(), Vec::new());
                a.push_lower(&mut la);
                b.push_lower(&mut lb);
                la == lb
            }
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
        assert_eq!(PathRules::Windows.parse("//server/share/a").unwrap(), PathRules::Windows.parse(r"\\server\share\a").unwrap());
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
    fn posix_names_differing_in_case_are_distinct_but_sort_together() {
        let lower = PathRules::Posix.folder_key(&names(&["pics"]));
        let upper = PathRules::Posix.folder_key(&names(&["Pics"]));
        assert_ne!(lower, upper);
        assert_ne!(PathRules::Posix.folder_key(&names(&["pics"])), PathRules::Posix.folder_key(&names(&["Pics", "x"])),);
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

    /// The order key of the file at `text` (a Windows path), as the catalog stores it.
    fn file_key(text: &str) -> Vec<u8> {
        let path = PathRules::Windows.parse(text).unwrap();
        let (folder, name) = PathRules::Windows.split_file_name(&path).unwrap();
        PathRules::Windows.order_key(&PathRules::Windows.folder_key(&folder.parts), &name)
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
    fn a_final_sigma_folder_scopes_its_files_under_the_writers_key() {
        // The writer stores a folder's key per character (Σ is σ, never ς); the scope reads it the same way.
        let file = file_key(r"D:\ΑΣ\a.jpg");
        let (lo, hi) = PathRules::Windows.scope_keys(&names(&["ΑΣ"]), false);
        assert!(file >= lo && hi.is_some_and(|hi| file < hi));
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
        let key = rules.order_key(&rules.folder_key(&below), &Name::from("x.jpg"));
        assert!(lo <= key && key < hi, "{key:?} not in {lo:?}..{hi:?}");
        // A sibling whose name extends the folder's is outside.
        let sibling = rules.order_key(&rules.folder_key(&names(&["photos\u{1}"])), &Name::from("x.jpg"));
        assert!(sibling >= hi);
    }
}

#[cfg(test)]
mod byte_tests {
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
}
