//! What kind of media a file is, judged by its name: which files the scanner records, and what the type filter
//! starts from. Whether an image is animated is only known after the import (`analysis.frames`).

/// Extensions recorded as images (decoded by `decode`).
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff"];
/// Extensions recorded as videos (FFmpeg: `video`).
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "mpg", "mpeg", "3gp"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
}

impl Kind {
    /// The catalog's `files.kind`.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        match key {
            "image" => Some(Kind::Image),
            "video" => Some(Kind::Video),
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
        IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).position(|e| *e == ext).map(|i| Ext(i as u8))
    }

    pub fn name(self) -> &'static str {
        let i = self.0 as usize;
        IMAGE_EXTENSIONS.get(i).unwrap_or_else(|| &VIDEO_EXTENSIONS[i - IMAGE_EXTENSIONS.len()])
    }

    pub fn kind(self) -> Kind {
        if (self.0 as usize) < IMAGE_EXTENSIONS.len() { Kind::Image } else { Kind::Video }
    }

    /// A file name's last extension, in any case (`a.mozjpeg.JPG` → `jpg`); `None` for a name without one, or one
    /// the library doesn't hold. Which files the scanner records.
    pub fn of_name(name: &str) -> Option<Ext> {
        let (_, ext) = name.rsplit_once('.')?;
        Ext::matching(ext.chars())
    }

    /// [`Ext::of_name`] of a name in UTF-16 (a directory listing's), read as `String::from_utf16_lossy` reads it
    /// (an unpaired surrogate is U+FFFD), so the scanner decodes only the names it records.
    pub fn of_name_utf16(name: &[u16]) -> Option<Ext> {
        let dot = name.iter().rposition(|&u| u == u16::from(b'.'))?;
        Ext::matching(
            char::decode_utf16(name[dot + 1..].iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)),
        )
    }

    /// The extension whose name is `ext` in lower case, without making it: lowercased character by character, as
    /// `str::to_lowercase` lowercases every character but Σ (which no extension has). So `M\u{212A}V` (KELVIN SIGN)
    /// is `mkv`, as it was when the scanner compared `to_lowercase()`.
    fn matching(ext: impl Iterator<Item = char> + Clone) -> Option<Ext> {
        let lower = ext.flat_map(char::to_lowercase);
        IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).position(|e| lower.clone().eq(e.chars())).map(|i| Ext(i as u8))
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
        let utf16 = |s: &str| s.encode_utf16().collect::<Vec<_>>();
        let names = ["a.mozjpeg.JPG", "README", "x.WebP", "clip.MP4", "notes.txt", "jpg", ".png", "a.", "a.jpg.txt"];
        let names = names.into_iter().chain(["clip.M\u{212A}V", "ΑΣ.ΣJPG", "İ.TIFF", "a.JPEG\u{0307}"]);
        for name in names {
            assert_eq!(Ext::of_name(name), reference(name), "{name}");
            assert_eq!(Ext::of_name_utf16(&utf16(name)), reference(name), "{name}");
        }
        assert_eq!(Ext::of_name("a.mozjpeg.JPG").map(Ext::name), Some("jpg"));
        assert_eq!(Ext::of_name("x.WebP").map(Ext::kind), Some(Kind::Image));
        assert_eq!(Ext::of_name("clip.MP4").map(Ext::kind), Some(Kind::Video));
        assert_eq!(Ext::of_name("clip.M\u{212A}V").map(Ext::name), Some("mkv"));
        assert_eq!(Ext::of_name("README"), None);
        assert_eq!(Ext::of_name("jpg"), None);
        // An unpaired surrogate before the dot is read as U+FFFD; the extension is still found.
        let mut lone = vec![0xD800];
        lone.extend(utf16(".Png"));
        assert_eq!(Ext::of_name_utf16(&lone), reference(&String::from_utf16_lossy(&lone)));
        assert_eq!(Ext::of_name_utf16(&lone).map(Ext::name), Some("png"));
        for ext in IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS) {
            let e = Ext::of(ext).unwrap();
            assert_eq!((e.name(), Ext::of_name(&format!("a.{}", ext.to_uppercase()))), (*ext, Some(e)));
        }
        assert_eq!(Ext::of("txt"), None);
        assert_eq!(Kind::from_key(Kind::Video.key()), Some(Kind::Video));
    }
}
