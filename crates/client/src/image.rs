//! The `image://` scheme: the URLs of the images the desktop's providers serve, built and read here alone. A request
//! names its provider and, after it, the image: an image key (`pg_model::image::ImageKey`, the app's interface) with
//! the thumbnail's height or the view, or a Yandex result's id.
//! - `image://thumb/<key>/<height>`
//! - `image://thumbhash/<key>`
//! - `image://full/<key>/<width>x<height>` (fitted), `…/full`, or `…/region/<x>,<y>,<w>,<h>/<width>x<height>` (the
//!   part of the image on screen while zoomed between fit and 1:1, `x`…`h` as fractions of the upright image)
//! - `image://yandex/<result id>` (its grid picture), or `…/original` (its file as the host gave it)

use pg_model::image::{ImageKey, View};
use pg_model::size::ThumbHeight;

/// The image providers, each registered under its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Thumb,
    Thumbhash,
    Full,
    Yandex,
}

impl Provider {
    /// Every provider.
    pub const ALL: [Provider; 4] = [Provider::Thumb, Provider::Thumbhash, Provider::Full, Provider::Yandex];

    /// Its name in the scheme: `image://<name>/…`.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Thumb => "thumb",
            Provider::Thumbhash => "thumbhash",
            Provider::Full => "full",
            Provider::Yandex => "yandex",
        }
    }

    /// The provider named `name`, if there is one.
    pub fn from_name(name: &str) -> Option<Provider> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// One image a provider serves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageRequest {
    /// The thumbnail `height` px high (within the thumbnail cache's heights, `pg_model::size::ThumbHeight`).
    Thumb { key: ImageKey, height: ThumbHeight },
    /// The ThumbHash placeholder.
    Thumbhash { key: ImageKey },
    /// The full image, in `view`.
    Full { key: ImageKey, view: View },
    /// A Yandex result's picture (`pg_app::yandex::Hit::id`): its grid picture, or the file as its host gave it.
    Yandex { id: u64, original: bool },
}

impl ImageRequest {
    /// The thumbnail of `key` at `height`.
    pub fn thumb(key: ImageKey, height: ThumbHeight) -> ImageRequest {
        ImageRequest::Thumb { key, height }
    }

    /// The provider the request is served by.
    pub fn provider(&self) -> Provider {
        match self {
            ImageRequest::Thumb { .. } => Provider::Thumb,
            ImageRequest::Thumbhash { .. } => Provider::Thumbhash,
            ImageRequest::Full { .. } => Provider::Full,
            ImageRequest::Yandex { .. } => Provider::Yandex,
        }
    }

    /// The part of the URL after the provider's name: `<key>/<height>`, `<key>`, `<key>/<view>`, or `<id>` and
    /// `<id>/original`.
    pub fn id(&self) -> String {
        match self {
            ImageRequest::Thumb { key, height } => format!("{key}/{}", height.get()),
            ImageRequest::Thumbhash { key } => key.to_string(),
            ImageRequest::Full { key, view } => format!("{key}/{}", view_path(view)),
            ImageRequest::Yandex { id, original: false } => id.to_string(),
            ImageRequest::Yandex { id, original: true } => format!("{id}/original"),
        }
    }

    /// The request's `image://<provider>/<id>` URL.
    pub fn url(&self) -> String {
        format!("image://{}/{}", self.provider().name(), self.id())
    }

    /// The request `provider` gets for `id` (the URL's part after the provider's name), or `None` when it doesn't
    /// read. A thumbnail's height is one the cache holds (`ThumbHeight`); another is refused.
    pub fn parse(provider: Provider, id: &str) -> Option<ImageRequest> {
        if provider == Provider::Yandex {
            let (id, original) = id.strip_suffix("/original").map_or((id, false), |id| (id, true));
            return Some(ImageRequest::Yandex { id: id.parse().ok()?, original });
        }
        let (key, rest) = id.split_once('/').unwrap_or((id, ""));
        let key = ImageKey::parse(key)?;
        match provider {
            Provider::Thumb => Some(ImageRequest::thumb(key, ThumbHeight::new(rest.parse().ok()?)?)),
            Provider::Thumbhash => Some(ImageRequest::Thumbhash { key }),
            Provider::Full => Some(ImageRequest::Full { key, view: view_of(rest)? }),
            Provider::Yandex => None,
        }
    }
}

/// The part of an `image://full/<key>/…` URL after the key: `full`, `<width>x<height>`, or
/// `region/<x>,<y>,<w>,<h>/<width>x<height>`. The fractions print in their shortest exact form, so [`view_of`] reads
/// them back to the same values.
pub fn view_path(view: &View) -> String {
    match *view {
        View::Full => "full".to_owned(),
        View::Fit(w, h) => format!("{w}x{h}"),
        View::Region((x, y, w, h), pw, ph) => format!("region/{x},{y},{w},{h}/{pw}x{ph}"),
    }
}

/// The view the part after the key names, or `None` when it doesn't read, or a region is out of the image. A region is
/// cut to the image, and its output size to 16384 px a side.
pub fn view_of(rest: &str) -> Option<View> {
    if rest == "full" {
        return Some(View::Full);
    }
    let Some(region) = rest.strip_prefix("region/") else {
        return size(rest).map(|(w, h)| View::Fit(w, h));
    };
    let (r, out) = region.split_once('/')?;
    let mut r = r.split(',').map(str::parse::<f64>);
    let (Some(Ok(x)), Some(Ok(y)), Some(Ok(w)), Some(Ok(h)), None) = (r.next(), r.next(), r.next(), r.next(), r.next())
    else {
        return None;
    };
    let (ow, oh) = size(out)?;
    let ok = (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y) && w > 0.0 && h > 0.0;
    ok.then_some(View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), ow.min(16384), oh.min(16384)))
}

/// `<width>x<height>` in device pixels.
fn size(text: &str) -> Option<(usize, usize)> {
    let (w, h) = text.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> ImageKey {
        ImageKey::parse(text).expect("a test key reads")
    }

    /// The request a URL names: its provider by name, then the rest.
    fn read(url: &str) -> Option<ImageRequest> {
        let (provider, id) = url.strip_prefix("image://")?.split_once('/')?;
        ImageRequest::parse(Provider::from_name(provider)?, id)
    }

    #[test]
    fn a_full_region_url_reads_back_to_its_request() {
        let text = "image://full/12@1700000000.0.2/region/0.25,0.5,0.5,0.25/400x300";
        let request = read(text).expect("reads");
        assert_eq!(request.url(), text);
        assert_eq!(
            request,
            ImageRequest::Full { key: key("12@1700000000.0.2"), view: View::Region((0.25, 0.5, 0.5, 0.25), 400, 300) }
        );
    }

    #[test]
    fn a_region_is_cut_to_the_image_and_its_output_capped() {
        let view = view_of("region/0.5,0.5,0.9,0.9/20000x7").expect("reads");
        assert_eq!(view, View::Region((0.5, 0.5, 0.5, 0.5), 16384, 7));
    }

    #[test]
    fn a_region_outside_the_image_does_not_read() {
        assert_eq!(view_of("region/1.0,0,0.5,0.5/10x10"), None);
        assert_eq!(view_of("region/0,0,0,0.5/10x10"), None);
        assert_eq!(view_of("region/0,0,0.5/10x10"), None);
    }

    #[test]
    fn thumbnail_heights_outside_the_cache_are_refused() {
        let at = |px| ThumbHeight::new(px).expect("a height in range");
        let ok = ImageRequest::parse(Provider::Thumb, "3/220").expect("reads");
        assert_eq!(ok, ImageRequest::Thumb { key: key("3"), height: at(220) });
        assert_eq!(ImageRequest::parse(Provider::Thumb, "3/5"), None, "below the cache's 16");
        assert_eq!(ImageRequest::parse(Provider::Thumb, "3/9999"), None, "above the cache's 2048");
        assert_eq!(ImageRequest::parse(Provider::Thumb, "3/tall"), None);
    }

    #[test]
    fn a_fitted_or_full_view_or_a_yandex_picture_reads_and_prints_back() {
        for text in [
            "image://full/7/1280x720",
            "image://full/7/full",
            "image://thumbhash/7",
            "image://yandex/4294967297",
            "image://yandex/4294967297/original",
        ] {
            assert_eq!(read(text).expect("reads").url(), text);
        }
        assert_eq!(read("image://yandex/9/original"), Some(ImageRequest::Yandex { id: 9, original: true }));
    }

    #[test]
    fn an_unknown_provider_or_key_does_not_read() {
        assert_eq!(Provider::from_name("video"), None);
        assert_eq!(ImageRequest::parse(Provider::Full, "not-a-number/full"), None);
        assert_eq!(ImageRequest::parse(Provider::Yandex, "x/original"), None);
    }

    #[test]
    fn each_provider_has_its_own_name() {
        for p in Provider::ALL {
            assert_eq!(Provider::from_name(p.name()), Some(p));
        }
    }
}
