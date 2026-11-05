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

    /// The path that user text names, if it is one of these rules' paths. A Windows drive with or without its
    /// root (`D:`, `D:\a`), a UNC path (`\\server\share\a`, anchor `\`, parts `server`, `share`, `a`), or a
    /// relative path (empty anchor). Empty parts (doubled or trailing separators) are dropped. Returns `None` for
    /// text that cannot be a path here (a NUL, which no name holds).
    pub fn parse(self, text: &str) -> Option<Path> {
        self.parse_bytes(text.as_bytes())
    }

    /// [`PathRules::parse`] over the bytes a path is stored as ([`PathRules::join`]): canonical WTF-8 on Windows,
    /// raw bytes on Posix. The separators are ASCII, so splitting at them keeps every part canonical.
    pub fn parse_bytes(self, bytes: &[u8]) -> Option<Path> {
        if bytes.contains(&0) {
            return None;
        }
        let sep = self.sep() as u8;
        let (anchor, rest): (&[u8], &[u8]) = match self {
            PathRules::Windows => {
                if let Some(rest) = bytes.strip_prefix(br"\\") {
                    (br"\", rest)
                } else if let [d, b':', ..] = bytes {
                    if d.is_ascii_alphabetic() { (&bytes[..2], trim_seps(&bytes[2..], sep)) } else { (b"", bytes) }
                } else {
                    (b"", bytes)
                }
            }
            PathRules::Posix => (b"", trim_seps(bytes, sep)),
        };
        let parts = rest.split(|&b| b == sep).filter(|p| !p.is_empty()).map(Name::from_raw).collect();
        Some(Path { anchor: Name::from_raw(anchor), parts })
    }

    /// A path's bytes, as the catalog stores them: the anchor, the root separator, then the parts joined by the
    /// separator. [`PathRules::parse_bytes`] reads them back.
    pub fn join(self, path: &Path) -> Vec<u8> {
        let sep = self.sep() as u8;
        let mut out = Vec::with_capacity(
            path.anchor.as_bytes().len() + 1 + path.parts.iter().map(|p| p.as_bytes().len() + 1).sum::<usize>(),
        );
        out.extend_from_slice(path.anchor.as_bytes());
        out.push(sep);
        for (i, part) in path.parts.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            out.extend_from_slice(part.as_bytes());
        }
        out
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
            // Below the folder, nothing sorts at or after 0xFF (no UTF-8 byte is 0xFF; raw Unix bytes never
            // start a name key with it either, as the key's 0x00 comes first).
            if lo.is_empty() { None } else { Some([lo.as_slice(), &[0xFF]].concat()) }
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
}

/// `bytes` without its leading separators.
fn trim_seps(bytes: &[u8], sep: u8) -> &[u8] {
    let n = bytes.iter().take_while(|&&b| b == sep).count();
    &bytes[n..]
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
