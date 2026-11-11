//! What kind of media a file is, judged by its name: which files the scanner records, and what the type filter
//! starts from. Whether an image is animated is only known after the import (`analysis.frames`).

/// Extensions recorded as images (decoded by `decode`).
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff"];
/// Extensions recorded as videos (FFmpeg: `video`).
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "mpg", "mpeg", "3gp"];

/// The catalog's `files.kind` of an image: the one spelling of the stored key, for Rust ([`Kind::key`]) and the SQL
/// that tests it (`concat!`).
#[macro_export]
macro_rules! kind_image {
    () => {
        "image"
    };
}

/// The catalog's `files.kind` of a video (see [`kind_image!`]).
#[macro_export]
macro_rules! kind_video {
    () => {
        "video"
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, strum::EnumIter)]
pub enum Kind {
    Image,
    Video,
}

impl Kind {
    /// The catalog's `files.kind`.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Image => kind_image!(),
            Kind::Video => kind_video!(),
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        match key {
            kind_image!() => Some(Kind::Image),
            kind_video!() => Some(Kind::Video),
            _ => None,
        }
    }
}

/// One of the extensions the library holds, as a byte: images first, then videos, in the lists' order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ext(u8);

impl Ext {
    /// A lower-case extension (`files.ext`); `None` if the library doesn't hold it.
    pub fn of(ext: &str) -> Option<Ext> {
        IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).position(|e| *e == ext).and_then(Ext::at)
    }

    /// The extension at a position in the two lists, as its byte; `None` past what a byte holds.
    fn at(position: usize) -> Option<Ext> {
        u8::try_from(position).ok().map(Ext)
    }

    pub fn name(self) -> &'static str {
        // Every `Ext` is a position in the two lists, so the name is always found.
        IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).nth(usize::from(self.0)).copied().unwrap_or_default()
    }

    pub fn kind(self) -> Kind {
        if usize::from(self.0) < IMAGE_EXTENSIONS.len() { Kind::Image } else { Kind::Video }
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
        Ext::matching(std::str::from_utf8(name.get(dot + 1..)?).ok()?.chars())
    }

    /// The extension whose name is `ext` in lower case, without making it: lowercased character by character, as
    /// `str::to_lowercase` lowercases every character but Σ (which no extension has). So `M\u{212A}V` (KELVIN SIGN)
    /// is `mkv`, as it was when the scanner compared `to_lowercase()`.
    fn matching(ext: impl Iterator<Item = char> + Clone) -> Option<Ext> {
        let lower = ext.flat_map(char::to_lowercase);
        IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).position(|e| lower.clone().eq(e.chars())).and_then(Ext::at)
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
        assert_eq!(Ext::of_name("a.mozjpeg.JPG").map(Ext::name), Some("jpg"));
        assert_eq!(Ext::of_name("x.WebP").map(Ext::kind), Some(Kind::Image));
        assert_eq!(Ext::of_name("clip.MP4").map(Ext::kind), Some(Kind::Video));
        assert_eq!(Ext::of_name("clip.M\u{212A}V").map(Ext::name), Some("mkv"));
        assert_eq!(Ext::of_name("README"), None);
        assert_eq!(Ext::of_name("jpg"), None);
        // An unpaired surrogate (WTF-8: three bytes no UTF-8 sequence holds) before the dot leaves the extension
        // found; one inside the extension is no extension the library holds.
        let lone = [&[0xED, 0xA0, 0x80][..], b".Png"].concat();
        assert_eq!(Ext::of_name_bytes(&lone).map(Ext::name), Some("png"));
        assert_eq!(Ext::of_name_bytes(b"a.p\xED\xA0\x80"), None);
        for ext in IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS) {
            let e = Ext::of(ext).unwrap();
            assert_eq!((e.name(), Ext::of_name(&format!("a.{}", ext.to_uppercase()))), (*ext, Some(e)));
        }
        assert_eq!(Ext::of("txt"), None);
        assert_eq!(Kind::from_key(Kind::Video.key()), Some(Kind::Video));
    }
}
