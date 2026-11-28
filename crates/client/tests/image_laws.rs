//! The laws of the `image://` scheme (`pg_client::image`), over generated requests: a request reads back from its URL,
//! and any text is read or refused without a panic.

use pg_client::image::{ImageRequest, Provider};
use pg_model::ids::FileId;
use pg_model::image::{ImageKey, Version, View};
use pg_model::turns::Turns;
use proptest::prelude::*;

fn turns() -> impl Strategy<Value = Turns> {
    any::<u8>().prop_map(Turns::new)
}

proptest! {
    // A request's URL is its provider and id; parsing them gives the request back. A region is generated in the
    // form `view_of` produces (cut to the image), so the round trip is exact.
    #[test]
    fn an_image_request_reads_back_from_its_url(
        file in any::<i64>(),
        versioned in any::<bool>(),
        mtime in any::<i64>(),
        rotation in turns(),
        pending in turns(),
        kind in 0u8..5,
        (x, y) in (0.0f64..1.0, 0.0f64..1.0),
        (w, h) in (0.001f64..=1.0, 0.001f64..=1.0),
        (pw, ph) in (1usize..=8192, 1usize..=8192),
        height in 16u32..=2048,
        id in any::<u64>(),
        original in any::<bool>(),
    ) {
        let version = versioned.then_some(Version { mtime, turns: rotation, pending_turns: pending });
        let key = ImageKey { file: FileId::new(file), version };
        let request = match kind {
            0 => ImageRequest::thumb(key, pg_model::size::ThumbHeight::new(height).expect("a height in the cache's range")),
            1 => ImageRequest::Thumbhash { key },
            2 => ImageRequest::Full { key, view: View::Fit(pw, ph) },
            3 => ImageRequest::Yandex { id, original },
            _ => ImageRequest::Full { key, view: View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), pw, ph) },
        };
        prop_assert_eq!(ImageRequest::parse(request.provider(), &request.id()), Some(request));
        let url = request.url();
        let (name, rest) = url.strip_prefix("image://").and_then(|u| u.split_once('/')).expect("a URL");
        prop_assert_eq!(Provider::from_key(name), Some(request.provider()));
        prop_assert_eq!(rest, request.id());
    }

    // Any provider and id are read or refused, never a panic.
    #[test]
    fn an_image_request_parse_never_panics(provider in 0usize..4, id in ".*") {
        let _ = ImageRequest::parse(Provider::ALL[provider], &id);
    }

    // A whole URL is read back to the request it was built from, and any text is read or refused, never a panic.
    #[test]
    fn a_whole_url_reads_back_to_its_request(id in any::<u64>(), text in ".*") {
        let request = ImageRequest::Yandex { id, original: true };
        prop_assert_eq!(ImageRequest::from_url(&request.url()), Some(request));
        let _ = ImageRequest::from_url(&text);
    }
}
