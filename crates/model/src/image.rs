//! The images the providers serve. An image key names a file's image, as the grid showed it when the key was made: its
//! text is `<file id>` or `<file id>@<mtime>.<rotation>.<turns>`, and the desktop's `image://` URLs carry it. A request
//! asks for one image of a key: a thumbnail, the ThumbHash placeholder, or the full image in a view.

use std::fmt;

use crate::ids::FileId;
use crate::turns::Turns;

/// What a key's version names beyond the file: the file as the grid showed it when the key was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    /// The file's modification time and the quarter turns it was stored with (the thumbnail cache's source).
    pub mtime: i64,
    pub rotation: Turns,
    /// The quarter turns clockwise asked for and not yet written then.
    pub turns: Turns,
}

impl Version {
    fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let mtime = parts.next()?.parse().ok()?;
        let rotation = Turns::new(parts.next()?.parse::<u8>().ok()?);
        let turns = Turns::new(parts.next()?.parse::<u8>().ok()?);
        Some(Version { mtime, rotation, turns })
    }
}

/// A file's image key. Without a version (or with one that doesn't read) it names the file's current image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageKey {
    pub file: FileId,
    pub version: Option<Version>,
}

impl ImageKey {
    /// The key `text` names, `None` when the file id doesn't read.
    pub fn parse(text: &str) -> Option<ImageKey> {
        let (file, version) = text.split_once('@').map_or((text, None), |(f, v)| (f, Some(v)));
        Some(ImageKey { file: FileId::new(file.parse().ok()?), version: version.and_then(Version::parse) })
    }
}

impl fmt::Display for ImageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.get())?;
        match self.version {
            Some(v) => write!(f, "@{}.{}.{}", v.mtime, v.rotation.get(), v.turns.get()),
            None => Ok(()),
        }
    }
}

/// What the viewer asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// Fitted into (width, height) device pixels, up or down.
    Fit(usize, usize),
    /// Full size (1:1 and beyond, and the loupe).
    Full,
    /// Zoom between fit and 1:1: the region (x, y, w, h, as fractions of the upright image) at (width, height) device
    /// pixels, the part of the zoomed image that is on screen.
    Region((f64, f64, f64, f64), usize, usize),
}

/// One image of a key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageRequest {
    /// The thumbnail `height` px high.
    Thumb { key: ImageKey, height: u32 },
    /// The ThumbHash placeholder.
    Thumbhash { key: ImageKey },
    /// The full image, in `view`.
    Full { key: ImageKey, view: View },
}
