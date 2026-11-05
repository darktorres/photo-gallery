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

/// The smallest and largest thumbnail height a request may ask for, in px.
pub const THUMB_HEIGHT: std::ops::RangeInclusive<u32> = 16..=2048;

impl View {
    /// The part of an `image://full/<key>/…` URL after the key: `full`, `<width>x<height>`, or
    /// `region/<x>,<y>,<w>,<h>/<width>x<height>`. The fractions print in their shortest exact form, so `parse` reads
    /// them back to the same values.
    pub fn path(&self) -> String {
        match *self {
            View::Full => "full".to_owned(),
            View::Fit(w, h) => format!("{w}x{h}"),
            View::Region((x, y, w, h), pw, ph) => format!("region/{x},{y},{w},{h}/{pw}x{ph}"),
        }
    }

    /// The view the part after the key names, or `None` when it doesn't read, or a region is out of the image. A
    /// region is cut to the image, and its output size to 16384 px a side.
    pub fn parse(rest: &str) -> Option<View> {
        if rest == "full" {
            return Some(View::Full);
        }
        let Some(region) = rest.strip_prefix("region/") else {
            return size(rest).map(|(w, h)| View::Fit(w, h));
        };
        let (r, out) = region.split_once('/')?;
        let mut r = r.split(',').map(str::parse::<f64>);
        let (Some(Ok(x)), Some(Ok(y)), Some(Ok(w)), Some(Ok(h)), None) =
            (r.next(), r.next(), r.next(), r.next(), r.next())
        else {
            return None;
        };
        let (ow, oh) = size(out)?;
        let ok = (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y) && w > 0.0 && h > 0.0;
        ok.then_some(View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), ow.min(16384), oh.min(16384)))
    }
}

/// `<width>x<height>` in device pixels.
fn size(text: &str) -> Option<(usize, usize)> {
    let (w, h) = text.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

impl ImageRequest {
    /// The provider the request is served by: its name in the `image://` scheme.
    pub fn provider(&self) -> &'static str {
        match self {
            ImageRequest::Thumb { .. } => "thumb",
            ImageRequest::Thumbhash { .. } => "thumbhash",
            ImageRequest::Full { .. } => "full",
        }
    }

    /// The part of the URL after the provider's name: `<key>/<height>`, `<key>`, or `<key>/<view>`.
    pub fn id(&self) -> String {
        match self {
            ImageRequest::Thumb { key, height } => format!("{key}/{height}"),
            ImageRequest::Thumbhash { key } => key.to_string(),
            ImageRequest::Full { key, view } => format!("{key}/{}", view.path()),
        }
    }

    /// The request's `image://<provider>/<id>` URL.
    pub fn url(&self) -> String {
        format!("image://{}/{}", self.provider(), self.id())
    }

    /// The request `provider` makes for `id` (the URL's part after the provider's name), or `None` when it doesn't
    /// read. A thumbnail's height is clamped to [`THUMB_HEIGHT`].
    pub fn parse(provider: &str, id: &str) -> Option<ImageRequest> {
        let (key, rest) = id.split_once('/').unwrap_or((id, ""));
        let key = ImageKey::parse(key)?;
        match provider {
            "thumb" => {
                let height = rest.parse::<u32>().ok()?;
                Some(ImageRequest::Thumb { key, height: height.clamp(*THUMB_HEIGHT.start(), *THUMB_HEIGHT.end()) })
            }
            "thumbhash" => Some(ImageRequest::Thumbhash { key }),
            "full" => Some(ImageRequest::Full { key, view: View::parse(rest)? }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> ImageKey {
        ImageKey::parse(text).expect("a test key reads")
    }

    #[test]
    fn a_full_region_url_reads_back_to_its_request() {
        let text = "image://full/12@1700000000.0.2/region/0.25,0.5,0.5,0.25/400x300";
        let (provider, id) = text.strip_prefix("image://").and_then(|t| t.split_once('/')).expect("a URL");
        let request = ImageRequest::parse(provider, id).expect("reads");
        assert_eq!(request.url(), text);
        assert_eq!(
            request,
            ImageRequest::Full { key: key("12@1700000000.0.2"), view: View::Region((0.25, 0.5, 0.5, 0.25), 400, 300) }
        );
    }

    #[test]
    fn a_region_is_cut_to_the_image_and_its_output_capped() {
        let view = View::parse("region/0.5,0.5,0.9,0.9/20000x7").expect("reads");
        assert_eq!(view, View::Region((0.5, 0.5, 0.5, 0.5), 16384, 7));
    }

    #[test]
    fn a_region_outside_the_image_does_not_read() {
        assert_eq!(View::parse("region/1.0,0,0.5,0.5/10x10"), None);
        assert_eq!(View::parse("region/0,0,0,0.5/10x10"), None);
        assert_eq!(View::parse("region/0,0,0.5/10x10"), None);
    }

    #[test]
    fn thumbnail_heights_are_clamped_and_bad_ones_refused() {
        let small = ImageRequest::parse("thumb", "3/5").expect("reads");
        assert_eq!(small, ImageRequest::Thumb { key: key("3"), height: 16 });
        let large = ImageRequest::parse("thumb", "3/9999").expect("reads");
        assert_eq!(large, ImageRequest::Thumb { key: key("3"), height: 2048 });
        assert_eq!(ImageRequest::parse("thumb", "3/tall"), None);
    }

    #[test]
    fn a_fitted_or_full_view_reads_and_prints_back() {
        for text in ["image://full/7/1280x720", "image://full/7/full", "image://thumbhash/7"] {
            let (provider, id) = text.strip_prefix("image://").and_then(|t| t.split_once('/')).expect("a URL");
            assert_eq!(ImageRequest::parse(provider, id).expect("reads").url(), text);
        }
    }

    #[test]
    fn an_unknown_provider_or_key_does_not_read() {
        assert_eq!(ImageRequest::parse("video", "7/full"), None);
        assert_eq!(ImageRequest::parse("full", "not-a-number/full"), None);
    }
}
