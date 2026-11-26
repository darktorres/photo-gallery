//! What kind of media a file is, judged by its name: which files the scanner records, and what the type filter
//! starts from. Whether an image is animated is only known after the import (`analysis.frames`).

use alloc::format;
use alloc::string::String;

use serde::{Deserialize, Serialize};

/// Every extension the library holds, with its kind, in code order: an extension's position is its stored code
/// (`files.ext`, [`Ext::code`]), so the list is append-only. A new extension goes at the end, and none is ever moved
/// or removed (its code would then name another extension in every catalog). Images are decoded by `decode`, videos by
/// FFmpeg (`video`).
const EXTENSIONS: &[(&str, Kind)] = &[
    ("jpg", Kind::Image),
    ("jpeg", Kind::Image),
    ("jfif", Kind::Image),
    ("png", Kind::Image),
    ("gif", Kind::Image),
    ("webp", Kind::Image),
    ("bmp", Kind::Image),
    ("tif", Kind::Image),
    ("tiff", Kind::Image),
    ("mp4", Kind::Video),
    ("m4v", Kind::Video),
    ("mov", Kind::Video),
    ("webm", Kind::Video),
    ("mkv", Kind::Video),
    ("avi", Kind::Video),
    ("wmv", Kind::Video),
    ("mpg", Kind::Video),
    ("mpeg", Kind::Video),
    ("3gp", Kind::Video),
];

/// The extensions' names, in code order.
fn names() -> impl Iterator<Item = &'static str> + Clone {
    EXTENSIONS.iter().map(|&(name, _)| name)
}

/// The catalog's `files.kind` (and `analysis.kind`) of an image, as the SQL literal of its code: the one spelling of the
/// stored code, for Rust ([`Kind::code`]) and the SQL that tests it (`concat!`).
#[macro_export]
macro_rules! kind_image {
    () => {
        "0"
    };
}

/// The catalog's `files.kind` of a video (see [`kind_image!`]).
#[macro_export]
macro_rules! kind_video {
    () => {
        "1"
    };
}

/// The kind of a file: an image or a video.
#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumIter)]
pub enum Kind {
    /// A still or animated image.
    Image,
    /// A video.
    Video,
}

impl Kind {
    /// The catalog's `files.kind` (and `analysis.kind`): 0 an image, 1 a video ([`kind_image!`], [`kind_video!`]).
    pub const fn code(self) -> i64 {
        match self {
            Kind::Image => 0,
            Kind::Video => 1,
        }
    }

    /// The kind the catalog's code stands for, if it is one.
    pub const fn from_code(code: i64) -> Option<Kind> {
        match code {
            0 => Some(Kind::Image),
            1 => Some(Kind::Video),
            _ => None,
        }
    }
}

/// Type, as the filter and the grid see it: the file's kind ([`Kind`]), with animated images apart from still ones.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
#[derive(strum::EnumString, strum::IntoStaticStr, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum MediaType {
    /// A still image.
    Image,
    /// An animated image.
    Animated,
    /// A video.
    Video,
}

impl MediaType {
    /// The name in JSON and in `CASE` (strum's snake_case, which serde's names are too).
    pub fn key(self) -> &'static str {
        self.into()
    }

    /// The media type the name stands for, if it is one.
    pub fn from_key(key: &str) -> Option<MediaType> {
        key.parse().ok()
    }
}

/// The extensions this build holds, images then videos (`jpg,jpeg,…;mp4,…`): the fingerprint of the set, which the
/// catalog keeps (`meta.held_exts`) so it can tell when a build with another set opened or scanned it.
pub fn held_exts() -> String {
    let of =
        |kind: Kind| EXTENSIONS.iter().filter(|e| e.1 == kind).map(|e| e.0).collect::<alloc::vec::Vec<_>>().join(",");
    format!("{};{}", of(Kind::Image), of(Kind::Video))
}

/// One of the extensions the library holds, as its code: its position in the extension table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ext(u8);

impl Ext {
    /// A lower-case extension (its name); `None` if the library doesn't hold it.
    pub fn of(ext: &str) -> Option<Ext> {
        names().position(|e| e == ext).and_then(Ext::at)
    }

    /// The extension at a position in the table, as its code; `None` past the table or what a byte holds.
    fn at(position: usize) -> Option<Ext> {
        if position >= EXTENSIONS.len() {
            return None;
        }
        u8::try_from(position).ok().map(Ext)
    }

    /// The extension's code, as the catalog stores it (`files.ext`): its position in the append-only table.
    pub fn code(self) -> i64 {
        i64::from(self.0)
    }

    /// The extension a stored code stands for, if it is one this build holds.
    pub fn from_code(code: i64) -> Option<Ext> {
        usize::try_from(code).ok().and_then(Ext::at)
    }

    /// The extension's name, without the dot.
    pub fn key(self) -> &'static str {
        // Every `Ext` is a position in the table, so the name is always found.
        EXTENSIONS.get(usize::from(self.0)).map_or("", |e| e.0)
    }

    /// The kind of file the extension is for.
    pub fn kind(self) -> Kind {
        EXTENSIONS.get(usize::from(self.0)).map_or(Kind::Image, |e| e.1)
    }

    /// A file name's last extension, in any case (`a.mozjpeg.JPG` → `jpg`); `None` for a name without one, or one
    /// the library doesn't hold. Which files the scanner records.
    pub fn of_name(name: &str) -> Option<Ext> {
        let (_, ext) = name.rsplit_once('.')?;
        Ext::matching(ext.chars())
    }

    /// [`Ext::of_name`] of a name's canonical bytes (`pg_model::name::Name`). An extension with no UTF-8 form (an
    /// unpaired surrogate, a byte no valid sequence holds) is none the library holds, so it is `None`.
    pub fn of_name_bytes(name: &[u8]) -> Option<Ext> {
        let dot = name.iter().rposition(|&b| b == b'.')?;
        let ext = name.get(dot + 1..)?;
        // An ASCII extension (nearly all) is compared as bytes, without decoding: the lists are lower-case ASCII, which is
        // what `char::to_lowercase` gives an ASCII character, so the two comparisons agree.
        if ext.is_ascii() {
            return names().position(|e| e.as_bytes().eq_ignore_ascii_case(ext)).and_then(Ext::at);
        }
        Ext::matching(core::str::from_utf8(ext).ok()?.chars())
    }

    /// The extension whose name is `ext` in lower case, without making it: lowercased character by character, as
    /// `str::to_lowercase` lowercases every character but Σ (which no extension has). So `M\u{212A}V` (KELVIN SIGN)
    /// is `mkv`, as it was when the scanner compared `to_lowercase()`.
    fn matching(ext: impl Iterator<Item = char> + Clone) -> Option<Ext> {
        let lower = ext.flat_map(char::to_lowercase);
        names().position(|e| lower.clone().eq(e.chars())).and_then(Ext::at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the scanner recorded before (`to_lowercase` of the last extension, looked up in the lists).
    fn reference(name: &str) -> Option<Ext> {
        name.rsplit_once('.').and_then(|(_, ext)| Ext::of(&ext.to_lowercase()))
    }

    #[test]
    fn extensions_by_name() {
        let names = ["a.mozjpeg.JPG", "README", "x.WebP", "clip.MP4", "notes.txt", "jpg", ".png", "a.", "a.jpg.txt"];
        let names = names.into_iter().chain(["clip.M\u{212A}V", "ΑΣ.ΣJPG", "İ.TIFF", "a.JPEG\u{0307}"]);
        for name in names {
            assert_eq!(Ext::of_name(name), reference(name), "{name}");
            assert_eq!(Ext::of_name_bytes(name.as_bytes()), reference(name), "{name}");
        }
        assert_eq!(Ext::of_name("a.mozjpeg.JPG").map(Ext::key), Some("jpg"));
        assert_eq!(Ext::of_name("x.WebP").map(Ext::kind), Some(Kind::Image));
        assert_eq!(Ext::of_name("clip.MP4").map(Ext::kind), Some(Kind::Video));
        assert_eq!(Ext::of_name("clip.M\u{212A}V").map(Ext::key), Some("mkv"));
        assert_eq!(Ext::of_name("README"), None);
        assert_eq!(Ext::of_name("jpg"), None);
        // An unpaired surrogate (WTF-8: three bytes no UTF-8 sequence holds) before the dot leaves the extension
        // found; one inside the extension is no extension the library holds.
        let lone = [&[0xED, 0xA0, 0x80][..], b".Png"].concat();
        assert_eq!(Ext::of_name_bytes(&lone).map(Ext::key), Some("png"));
        assert_eq!(Ext::of_name_bytes(b"a.p\xED\xA0\x80"), None);
        for ext in super::names() {
            let e = Ext::of(ext).unwrap();
            assert_eq!((e.key(), Ext::of_name(&format!("a.{}", ext.to_uppercase()))), (ext, Some(e)));
        }
        assert_eq!(Ext::of("txt"), None);
        assert_eq!(Kind::from_code(Kind::Video.code()), Some(Kind::Video));
    }

    /// The stored codes are pinned: an extension's code is in every catalog, so the table only grows at its end. A
    /// change here that is not an appended row breaks every catalog's `files.ext`.
    #[test]
    fn the_stored_codes_are_pinned() {
        let pinned = [
            "jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff", "mp4", "m4v", "mov", "webm", "mkv",
            "avi", "wmv", "mpg", "mpeg", "3gp",
        ];
        for (code, name) in pinned.iter().enumerate() {
            let code = i64::try_from(code).unwrap();
            assert_eq!(Ext::from_code(code).map(Ext::key), Some(*name), "code {code}");
            assert_eq!(Ext::of(name).map(Ext::code), Some(code), "{name}");
        }
        assert_eq!(Ext::from_code(-1), None);
        assert_eq!(Ext::from_code(i64::try_from(EXTENSIONS.len()).unwrap()), None);
        assert_eq!((Kind::Image.code(), Kind::Video.code()), (0, 1));
        assert_eq!((kind_image!(), kind_video!()), ("0", "1"));
        assert_eq!(held_exts(), "jpg,jpeg,jfif,png,gif,webp,bmp,tif,tiff;mp4,m4v,mov,webm,mkv,avi,wmv,mpg,mpeg,3gp");
    }
}
