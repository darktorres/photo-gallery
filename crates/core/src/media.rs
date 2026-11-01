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
}

/// A name's last extension in lower case (`a.mozjpeg.JPG` → `jpg`); empty when it has none.
pub fn extension(name: &str) -> String {
    name.rsplit_once('.').map_or(String::new(), |(_, ext)| ext.to_lowercase())
}

/// What the file is by its extension; `None` for files the library doesn't hold.
pub fn kind_of(name: &str) -> Option<Kind> {
    let ext = extension(name);
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(extension("a.mozjpeg.JPG"), "jpg");
        assert_eq!(extension("README"), "");
        assert_eq!(kind_of("x.WebP"), Some(Kind::Image));
        assert_eq!(kind_of("clip.MP4"), Some(Kind::Video));
        assert_eq!(kind_of("notes.txt"), None);
        for ext in IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS) {
            let e = Ext::of(ext).unwrap();
            assert_eq!((e.name(), Some(e.kind())), (*ext, kind_of(&format!("a.{ext}"))));
        }
        assert_eq!(Ext::of("txt"), None);
        assert_eq!(kind_of("jpg"), None);
        assert_eq!(Kind::from_key(Kind::Video.key()), Some(Kind::Video));
    }
}
