//! Laws of the pure model types, checked over generated inputs (S-18.2): the quarter-turn group, the EXIF orientation
//! as a mirror plus turns, and the image key's text round trip. Each law is one the code's docs state.

use pg_model::ids::FileId;
use pg_model::image::{ImageKey, ImageRequest, Version, View};
use pg_model::orientation::Orientation;
use pg_model::turns::Turns;
use proptest::prelude::*;

fn turns() -> impl Strategy<Value = Turns> {
    any::<u8>().prop_map(Turns::new)
}

fn orientation() -> impl Strategy<Value = Orientation> {
    // 1 to 8 are the orientations, so `from_tag` gives each tag's own (`a_broken_tag_is_upright` checks that).
    (1u8..=8).prop_map(Orientation::from_tag)
}

proptest! {
    // The quarter turns form the group of four: addition modulo four.
    #[test]
    fn turns_add_as_a_group(a in turns(), b in turns(), c in turns()) {
        prop_assert_eq!(a.plus(b), b.plus(a));
        prop_assert_eq!(a.plus(b).plus(c), a.plus(b.plus(c)));
        prop_assert_eq!(a.plus(Turns::NONE), a);
        prop_assert_eq!(a.plus(Turns::new(4 - a.get())), Turns::NONE);
    }

    // A value is always 0 to 3: n and n + 4 are the same turn.
    #[test]
    fn turns_are_taken_modulo_four(n in any::<u8>()) {
        prop_assert!(Turns::new(n).get() < 4);
        prop_assert_eq!(Turns::new(n), Turns::new(n % 4));
    }

    // A quarter turn (one or three) swaps the picture's width and height; a half or none does not.
    #[test]
    fn turns_swap_axes_exactly_when_odd(t in turns()) {
        prop_assert_eq!(t.swaps_axes(), t.get() % 2 == 1);
    }

    // The orientation is its mirror and turns, and back: each tag's parts rebuild it.
    #[test]
    fn orientation_parts_rebuild_the_orientation(o in orientation()) {
        let (mirrored, t) = o.parts();
        prop_assert_eq!(Orientation::from_parts(mirrored, t), o);
    }

    // Every mirror and turn pair is one of the eight orientations, and its parts are that pair.
    #[test]
    fn from_parts_then_parts_is_the_identity(mirrored in any::<bool>(), t in turns()) {
        prop_assert_eq!(Orientation::from_parts(mirrored, t).parts(), (mirrored, t));
    }

    // Turning a shown picture further adds to its turns and keeps its mirror.
    #[test]
    fn compose_adds_turns_and_keeps_the_mirror(o in orientation(), t in turns()) {
        let (mirrored, before) = o.parts();
        let (mirrored_after, after) = o.compose(t).parts();
        prop_assert_eq!(mirrored_after, mirrored);
        prop_assert_eq!(after, before.plus(t));
    }

    // The orientation swaps the axes exactly when its turns do.
    #[test]
    fn orientation_swaps_axes_exactly_when_its_turns_do(o in orientation()) {
        prop_assert_eq!(o.swaps_axes(), o.parts().1.swaps_axes());
    }

    // A tag outside 1 to 8 shows the picture as stored, whatever it is.
    #[test]
    fn a_broken_tag_is_upright(tag in any::<u8>()) {
        if (1..=8).contains(&tag) {
            prop_assert_eq!(Orientation::from_tag(tag).get(), tag);
        } else {
            prop_assert_eq!(Orientation::from_tag(tag), Orientation::UPRIGHT);
            prop_assert!(Orientation::new(tag).is_none());
        }
    }

    // A key's text is its file id, and with a version `@mtime.rotation.turns`; parsing gives the key back.
    #[test]
    fn image_key_text_round_trips(
        file in any::<i64>(),
        versioned in any::<bool>(),
        mtime in any::<i64>(),
        rotation in turns(),
        pending in turns(),
    ) {
        let version = versioned.then_some(Version { mtime, rotation, turns: pending });
        let key = ImageKey { file: FileId::new(file), version };
        prop_assert_eq!(ImageKey::parse(&key.to_string()), Some(key));
    }

    // Any text is read or refused, never a panic.
    #[test]
    fn image_key_parse_never_panics(text in ".*") {
        let _ = ImageKey::parse(&text);
    }

    // A request's URL is its provider and id; parsing them gives the request back. A region is generated in the
    // form `View::parse` produces (cut to the image), so the round trip is exact.
    #[test]
    fn an_image_request_reads_back_from_its_url(
        file in any::<i64>(),
        versioned in any::<bool>(),
        mtime in any::<i64>(),
        rotation in turns(),
        pending in turns(),
        kind in 0u8..4,
        (x, y) in (0.0f64..1.0, 0.0f64..1.0),
        (w, h) in (0.001f64..=1.0, 0.001f64..=1.0),
        (pw, ph) in (1usize..=16384, 1usize..=16384),
        height in 16u32..=2048,
    ) {
        let version = versioned.then_some(Version { mtime, rotation, turns: pending });
        let key = ImageKey { file: FileId::new(file), version };
        let request = match kind {
            0 => ImageRequest::Thumb { key, height },
            1 => ImageRequest::Thumbhash { key },
            2 => ImageRequest::Full { key, view: View::Fit(pw, ph) },
            _ => ImageRequest::Full { key, view: View::Region((x, y, w.min(1.0 - x), h.min(1.0 - y)), pw, ph) },
        };
        prop_assert_eq!(ImageRequest::parse(request.provider(), &request.id()), Some(request));
    }

    // Any provider and id are read or refused, never a panic.
    #[test]
    fn an_image_request_parse_never_panics(provider in ".*", id in ".*") {
        let _ = ImageRequest::parse(&provider, &id);
    }
}
