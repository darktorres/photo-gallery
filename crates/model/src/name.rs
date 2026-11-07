//! A file or folder name, kept exactly as the OS has it. A Windows name is UTF-16 and may hold unpaired surrogates
//! (NTFS allows them, Win32 mostly does not); a Unix name is any bytes but NUL. Both are stored as bytes in one
//! canonical form: UTF-8 for every scalar value, with an unpaired surrogate written as its three-byte generalised
//! UTF-8 (WTF-8, `ED A0..BF 80..BF`). Two names are equal exactly when their bytes are, so a name can key a table
//! and be stored as a BLOB without losing anything.
//!
//! Only display (a lossy `&str` for the UI and messages) and user-typed text cross to `&str`; the exact form goes
//! to the OS through the platform's native buffer.

use std::borrow::Cow;
use std::fmt;

/// One unit of a name: a scalar value, an unpaired surrogate (WTF-8 only), or a byte that is not part of any
/// valid sequence (a Unix name).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Char(char),
    Surrogate(u16),
    Byte(u8),
}

/// The name's bytes, canonical as the module says.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(Box<[u8]>);

impl Name {
    /// The name of UTF-16 units, as Windows has them. A high surrogate followed by a low one is one scalar value;
    /// any other surrogate is kept as itself (WTF-8), so nothing is replaced.
    pub fn from_wide(units: &[u16]) -> Name {
        let mut bytes = Vec::with_capacity(units.len() * 2);
        let mut i = 0;
        while let Some(&u) = units.get(i) {
            let pair = match (u, units.get(i + 1)) {
                (0xD800..=0xDBFF, Some(&lo @ 0xDC00..=0xDFFF)) => {
                    char::from_u32(0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(lo) - 0xDC00))
                }
                _ => None,
            };
            match pair {
                Some(c) => {
                    push_scalar(&mut bytes, c);
                    i += 2;
                }
                None => {
                    push_code_point(&mut bytes, u);
                    i += 1;
                }
            }
        }
        Name(bytes.into())
    }

    /// The bytes of a Unix name, exactly as the system gave them.
    pub fn from_raw(bytes: &[u8]) -> Name {
        Name(bytes.into())
    }

    /// This name with ASCII `prefix` and `suffix` around it: the same bytes in the middle, so a name with no UTF-8 form
    /// keeps its exact bytes (WTF-8 on Windows stays canonical, since ASCII joins nothing). Used for a file's temp name.
    pub fn with_affixes(&self, prefix: &str, suffix: &str) -> Name {
        debug_assert!(prefix.is_ascii() && suffix.is_ascii(), "affixes are ASCII");
        let mut bytes = Vec::with_capacity(prefix.len() + self.0.len() + suffix.len());
        bytes.extend_from_slice(prefix.as_bytes());
        bytes.extend_from_slice(&self.0);
        bytes.extend_from_slice(suffix.as_bytes());
        Name(bytes.into())
    }

    /// The canonical bytes of a Windows name (WTF-8), or `None` when they are not in that form (a byte that is in no
    /// valid sequence, or a surrogate pair written as two surrogates). A catalog read back through this is exact.
    pub fn from_wtf8(bytes: &[u8]) -> Option<Name> {
        let mut wide = Vec::with_capacity(bytes.len());
        for unit in units(bytes) {
            match unit {
                Unit::Char(c) => {
                    let mut buf = [0u16; 2];
                    wide.extend_from_slice(c.encode_utf16(&mut buf));
                }
                Unit::Surrogate(s) => wide.push(s),
                Unit::Byte(_) => return None,
            }
        }
        let name = Name::from_wide(&wide);
        (name.0.as_ref() == bytes).then_some(name)
    }

    /// The exact bytes, for storage and for keys.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The units of the name, in order.
    pub fn units(&self) -> impl Iterator<Item = Unit> + '_ {
        units(&self.0)
    }

    /// The UTF-16 units of the name, for the Windows API. A Unix name's [`Unit::Byte`]s are widened as they are
    /// (not meaningful on Windows, where a name has no bytes outside WTF-8).
    pub fn to_wide(&self) -> Vec<u16> {
        let mut wide = Vec::with_capacity(self.0.len());
        for unit in self.units() {
            match unit {
                Unit::Char(c) => {
                    let mut buf = [0u16; 2];
                    wide.extend_from_slice(c.encode_utf16(&mut buf));
                }
                Unit::Surrogate(s) => wide.push(s),
                Unit::Byte(b) => wide.push(u16::from(b)),
            }
        }
        wide
    }

    /// The name for display, lossy: each unit that is not a valid character shows as U+FFFD. Never for storage.
    pub fn display(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    /// The name's order key part, appended to `key`: 0x00, the name in lower case (each scalar value by Unicode's
    /// lower case, an unpaired surrogate or a stray byte as its own bytes), 0x00, then the exact bytes. The
    /// case-insensitive part sorts the names; the exact part makes the order total. No name holds a NUL, so the
    /// second 0x00 is unambiguous and the key is injective.
    pub fn push_name_key(&self, key: &mut Vec<u8>) {
        key.push(0x00);
        self.push_lower(key);
        key.push(0x00);
        key.extend_from_slice(&self.0);
    }

    /// The name in lower case, appended: each scalar value by Unicode's lower case, an unpaired surrogate or a stray
    /// byte as its own bytes. Case-insensitive comparison and the case-insensitive part of the name key use this.
    pub fn push_lower(&self, out: &mut Vec<u8>) {
        let mut buf = [0u8; 4];
        for unit in self.units() {
            match unit {
                Unit::Char(c) => {
                    for lower in c.to_lowercase() {
                        out.extend_from_slice(lower.encode_utf8(&mut buf).as_bytes());
                    }
                }
                Unit::Surrogate(s) => push_code_point(out, s),
                Unit::Byte(b) => out.push(b),
            }
        }
    }

    /// [`Name::push_name_key`] as a new vector.
    pub fn name_key(&self) -> Vec<u8> {
        let mut key = Vec::with_capacity(self.0.len() * 2 + 2);
        self.push_name_key(&mut key);
        key
    }

    /// Whether the name is `.` or `..` (never listed as a file or folder).
    pub fn is_dot(&self) -> bool {
        matches!(self.0.as_ref(), b"." | b"..")
    }
}

/// The units of canonical or raw name bytes. Anything that is not a valid sequence (or a generalised surrogate)
/// comes out as one [`Unit::Byte`], so decoding never fails and never skips a byte.
fn units(bytes: &[u8]) -> impl Iterator<Item = Unit> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        let lead = *bytes.get(i)?;
        let len = match lead {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 0,
        };
        let seq = if len > 0 { bytes.get(i..i + len) } else { None };
        if let Some(seq) = seq {
            if let Ok(s) = std::str::from_utf8(seq) {
                i += len;
                return s.chars().next().map(Unit::Char);
            }
            if let &[b0 @ 0xED, b1 @ 0xA0..=0xBF, b2 @ 0x80..=0xBF] = seq {
                i += 3;
                let cp = (u16::from(b0 & 0x0F) << 12) | (u16::from(b1 & 0x3F) << 6) | u16::from(b2 & 0x3F);
                return Some(Unit::Surrogate(cp));
            }
        }
        i += 1;
        Some(Unit::Byte(lead))
    })
}

/// A scalar value's UTF-8 bytes, appended.
fn push_scalar(bytes: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// A code point of one UTF-16 unit appended: a scalar value as UTF-8, a surrogate as generalised UTF-8 (WTF-8).
fn push_code_point(bytes: &mut Vec<u8>, cp: u16) {
    match char::from_u32(u32::from(cp)) {
        Some(c) => push_scalar(bytes, c),
        None => {
            // A surrogate: 1110 xxxx 10xxxxxx 10xxxxxx, the three-byte form UTF-8 reserves for scalar values.
            bytes.extend_from_slice(&[
                0xE0 | u8::try_from(cp >> 12).unwrap_or(u8::MAX),
                0x80 | u8::try_from((cp >> 6) & 0x3F).unwrap_or(u8::MAX),
                0x80 | u8::try_from(cp & 0x3F).unwrap_or(u8::MAX),
            ]);
        }
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Name {
        Name(s.as_bytes().into())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({:?})", self.display())
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// A name is a JSON string when its bytes are valid UTF-8 (so the JSON stays readable), else `{"b": "<hex>"}`.
impl serde::Serialize for Name {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match std::str::from_utf8(&self.0) {
            Ok(s) => serializer.serialize_str(s),
            Err(_) => {
                let mut st = serializer.serialize_struct("Name", 1)?;
                st.serialize_field("b", &hex_encode(&self.0))?;
                st.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Name {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Name, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Text(String),
            Bytes { b: String },
        }
        match Repr::deserialize(deserializer)? {
            Repr::Text(s) => Ok(Name::from(s.as_str())),
            Repr::Bytes { b } => {
                let bytes = hex_decode(&b).ok_or_else(|| serde::de::Error::custom("the name's bytes are not hex"))?;
                Ok(Name::from_raw(&bytes))
            }
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        // Each nibble is below 16, so each is a digit (lower case).
        out.extend(char::from_digit(u32::from(b >> 4), 16));
        out.extend(char::from_digit(u32::from(b & 0x0F), 16));
    }
    out
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    text.as_bytes().chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paired_surrogate_is_one_scalar_and_a_lone_one_is_kept() {
        let emoji = Name::from_wide(&[0xD83D, 0xDE00]);
        assert_eq!(emoji, Name::from("😀"));
        let lone = Name::from_wide(&[0x61, 0xD800, 0x62]);
        assert_eq!(lone.as_bytes(), &[0x61, 0xED, 0xA0, 0x80, 0x62]);
        assert_eq!(lone.to_wide(), vec![0x61, 0xD800, 0x62]);
        assert_eq!(lone.display(), "a\u{FFFD}\u{FFFD}\u{FFFD}b");
    }

    /// A temp name keeps the original's exact bytes between its ASCII affixes, a lone surrogate included, and stays
    /// canonical WTF-8 (what `Name::from_wtf8` reads back).
    #[test]
    fn with_affixes_keeps_the_bytes_between_ascii_affixes() {
        let lone = Name::from_wide(&[0x61, 0xD800, 0x62, 0x2E, 0x6A, 0x70, 0x67]);
        let temp = lone.with_affixes(".", ".pg-write");
        assert_eq!(temp.as_bytes(), b".a\xED\xA0\x80b.jpg.pg-write");
        assert_eq!(Name::from_wtf8(temp.as_bytes()), Some(temp));
        assert_eq!(Name::from("a.jpg").with_affixes(".", ".pg-write"), Name::from(".a.jpg.pg-write"));
    }

    #[test]
    fn a_split_pair_is_not_canonical() {
        // Two surrogates written as two three-byte forms are not the emoji's bytes.
        assert_eq!(Name::from_wtf8(&[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]), None);
        assert_eq!(Name::from_wtf8(&[0xF0, 0x9F, 0x98, 0x80]), Some(Name::from("😀")));
    }

    #[test]
    fn stray_bytes_come_out_as_bytes() {
        let raw = Name::from_raw(b"a\xFF\xC3");
        let units: Vec<Unit> = raw.units().collect();
        assert_eq!(units, vec![Unit::Char('a'), Unit::Byte(0xFF), Unit::Byte(0xC3)]);
        assert_eq!(Name::from_wtf8(b"a\xFF"), None);
    }

    /// The name key of text is pinned: 0x00, the name with each character lowered alone, 0x00, the exact bytes. So a Σ
    /// is σ wherever it stands (never the final ς of `str::to_lowercase`); İ lowers to two characters, ẞ to ß.
    #[test]
    fn the_name_key_of_text_is_pinned() {
        let key = |s: &str| Name::from(s).name_key();
        let pinned = |lower: &str, exact: &str| [&[0x00][..], lower.as_bytes(), &[0x00], exact.as_bytes()].concat();
        assert_eq!(key("Plain.JPG"), pinned("plain.jpg", "Plain.JPG"));
        assert_eq!(key("Σίσυφος"), pinned("σίσυφος", "Σίσυφος"));
        assert_eq!(key("İstanbul"), pinned("i\u{307}stanbul", "İstanbul"));
        assert_eq!(key("ẞ"), pinned("ß", "ẞ"));
        assert_eq!(key("a\u{1}b\u{2}c"), pinned("a\u{1}b\u{2}c", "a\u{1}b\u{2}c"));
        assert_eq!(key("😀 x"), pinned("😀 x", "😀 x"));
    }

    #[test]
    fn serde_is_a_string_for_text_and_hex_otherwise() {
        let text = Name::from("photo.jpg");
        assert_eq!(serde_json::to_string(&text).unwrap(), r#""photo.jpg""#);
        let lone = Name::from_wide(&[0xD800]);
        let json = serde_json::to_string(&lone).unwrap();
        assert_eq!(json, r#"{"b":"eda080"}"#);
        assert_eq!(serde_json::from_str::<Name>(&json).unwrap(), lone);
        assert_eq!(serde_json::from_str::<Name>(r#""photo.jpg""#).unwrap(), text);
    }
}
