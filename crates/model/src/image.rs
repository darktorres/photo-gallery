//! The images of a file the app serves. An image key names a file's image, as the grid showed it when the key was made:
//! its text is `<file id>` or `<file id>@<mtime>.<rotation>.<turns>`. A view is how the full image is asked for. The
//! URLs that carry them are the front end's (`pg_client::image`).

use std::fmt;

use crate::ids::FileId;
use crate::turns::Turns;

/// What a key's version names beyond the file: the file as the grid showed it when the key was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    /// The file's modification time and the quarter turns it was stored with (the thumbnail cache's source).
    pub mtime: i64,
    pub turns: Turns,
    /// The quarter turns clockwise asked for and not yet written then (`pending_turns`, as `FileState` and the grid name them).
    pub pending_turns: Turns,
}

impl Version {
    fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let mtime = parts.next()?.parse().ok()?;
        let rotation = Turns::new(parts.next()?.parse::<u8>().ok()?);
        let pending_turns = Turns::new(parts.next()?.parse::<u8>().ok()?);
        Some(Version { mtime, turns: rotation, pending_turns })
    }
}

/// A file's image key. Without a version (or with one that doesn't read) it names the file's current image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageKey {
    pub file: FileId,
    pub version: Option<Version>,
}

impl ImageKey {
    /// The key `text` names, `None` when the file id doesn't read or the version after `@` is not one. An unreadable
    /// version is not the same as no version: it names a picture (a version) this key cannot show, so the key is refused
    /// rather than served as the current file.
    pub fn parse(text: &str) -> Option<ImageKey> {
        let (file, version) = text.split_once('@').map_or((text, None), |(f, v)| (f, Some(v)));
        let version = match version {
            Some(text) => Some(Version::parse(text)?),
            None => None,
        };
        Some(ImageKey { file: FileId::new(file.parse().ok()?), version })
    }
}

impl fmt::Display for ImageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.get())?;
        match self.version {
            Some(v) => write!(f, "@{}.{}.{}", v.mtime, v.turns.get(), v.pending_turns.get()),
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
