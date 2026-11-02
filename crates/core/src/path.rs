//! Windows paths as the catalog and the tree compare them (model-shaped: pure string and byte work, no catalog or
//! Windows calls): the merged path of a folder (its drive dropped, so every drive's copy is one folder), the byte
//! keys that order folders and files by name (case-insensitive, exact after that, so the order is total), and
//! whether a path is inside another.

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
