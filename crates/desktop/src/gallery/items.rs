//! What the model says of one item: its key and URLs, its size and turns, its rating and badge, its borders, its
//! media type and its status-bar line.

use std::io::Write;

use cxx_qt_lib::{QRectF, QString, QStringList};
use pg_client::image::ImageRequest;
use pg_client::layout;
use pg_model::detection::{Class, Edge};
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::image::{ImageKey, Version};
use pg_model::media::MediaType;
use pg_model::orientation::Orientation;
use pg_model::size::ThumbHeight;
use pg_model::turns::Turns;
use pg_wording::FoundBorders;

use super::qobject;
use crate::bridges::gallery_state::Detail;

/// What Qt's players add to show a file upright: the quarter turns clockwise, and whether the frames are mirrored first
/// (`player_turns`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PlayerTurns {
    pub(super) turns: i32,
    pub(super) mirrored: bool,
}

impl PlayerTurns {
    /// The turns and mirror for a file of `media`, with EXIF orientation `exif` and `turned` catalog and pending turns
    /// (`player_turns`). An animation takes the orientation (its mirror, then the turns); a video keeps the turns alone.
    pub(super) fn of(media: MediaType, exif: Orientation, turned: Turns) -> PlayerTurns {
        match media {
            MediaType::Animated => {
                let (mirrored, turns) = exif.compose(turned).parts();
                PlayerTurns { turns: i32::from(turns.get()), mirrored }
            }
            _ => PlayerTurns { turns: i32::from(turned.get()), mirrored: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An animation is turned by its whole orientation: the mirror, then the quarter turns, for each of the eight EXIF
    // orientations, with the catalog and pending turns added on. A video is never mirrored (Qt's player turns it).
    #[test]
    fn an_animation_is_turned_by_its_orientation_and_the_turns_on_top() {
        for tag in 1..=8 {
            let exif = Orientation::new(tag).unwrap();
            for turned in 0..4 {
                let got = PlayerTurns::of(MediaType::Animated, exif, Turns::new(turned));
                let (mirrored, turns) = exif.compose(Turns::new(turned)).parts();
                assert_eq!((got.mirrored, got.turns), (mirrored, i32::from(turns.get())), "tag {tag}, turned {turned}");
            }
        }
    }

    #[test]
    fn a_video_takes_the_turns_alone() {
        for tag in 1..=8 {
            let exif = Orientation::new(tag).unwrap();
            let got = PlayerTurns::of(MediaType::Video, exif, Turns::new(3));
            assert_eq!(got, PlayerTurns { turns: 3, mirrored: false }, "tag {tag}");
        }
    }
}

impl qobject::GalleryModel {
    /// The image's aspect as shown (pending turns included); 1 without its height (`ROLE_IMAGE_ASPECTS`).
    pub(super) fn image_aspect(&self, item: i32) -> f64 {
        let (w, h) = (self.full_width(item), self.full_height(item));
        if h > 0 { f64::from(w) / f64::from(h) } else { 1.0 }
    }

    pub(crate) fn file_id(&self, item: i32) -> i64 {
        self.file_at(item).map_or(-1, FileId::get)
    }

    pub(crate) fn index_of(&self, file_id: i64) -> i32 {
        crate::rows::to_row(self.grid.position_of(FileId::new(file_id)))
    }

    /// Width ÷ height, as laid out (`ROLE_ASPECTS`).
    pub(super) fn aspect(&self, item: i32) -> f64 {
        self.entry(item).map_or(1.0, |i| layout::clamp_aspect(i.aspect()))
    }

    /// What names the item's image for the providers (`pg_model::image::ImageKey`, whose text layout is documented there):
    /// it changes whenever the image does, so no stale image is shown.
    pub(super) fn image_key(&self, item: i32) -> QString {
        self.entry(item).map_or_else(QString::default, |e| {
            let key = ImageKey {
                file: e.id,
                version: Some(Version { mtime: e.mtime, turns: e.turns, pending_turns: e.pending_turns }),
            };
            // Formatted on the stack: the id and mtime at most 20 bytes each, the rotation and the turns one digit each
            // (0–3), so 20 + 1 + 20 + 1 + 1 + 1 + 1 = 45 bytes.
            let mut buf = [0u8; 64];
            let mut rest = &mut buf[..];
            if write!(rest, "{key}").is_err() {
                return QString::default();
            }
            let len = 64 - rest.len();
            let Some(Ok(text)) = buf.get(..len).map(std::str::from_utf8) else { return QString::default() };
            QString::from(text)
        })
    }

    /// The image keys of the items up to `reach` before and after `item`, nearest first, for the viewer to preload.
    pub(super) fn keys_around(&self, item: i32, reach: i32) -> QStringList {
        if self.entry(item).is_none() {
            return QStringList::default();
        }
        (1..=reach)
            .flat_map(|d| [item - d, item + d])
            .filter(|&i| self.entry(i).is_some())
            .map(|i| self.image_key(i))
            .collect()
    }

    /// The URL of the full-size image of a file, the loupe's and the 1:1 view's (`image://full/<key>/full`).
    pub(super) fn full_url(&self, key: &QString) -> QString {
        crate::viewport::full_url(key, pg_model::image::View::Full)
    }

    /// The URL of a file's thumbnail at `height` (`image://thumb/<key>/<height>`), or empty when there is no height yet or
    /// the key doesn't read (the row then shows its thumbhash).
    pub(super) fn thumb_url(&self, key: &QString, height: Option<ThumbHeight>) -> QString {
        match (ImageKey::parse(&key.to_string()), height) {
            (Some(key), Some(height)) => QString::from(&ImageRequest::thumb(key, height).url()),
            _ => QString::default(),
        }
    }

    /// The URL of a file's placeholder (`image://thumbhash/<key>`), or empty when the key doesn't read.
    pub(super) fn thumbhash_url(&self, key: &QString) -> QString {
        ImageKey::parse(&key.to_string())
            .map_or_else(QString::default, |key| QString::from(&ImageRequest::Thumbhash { key }.url()))
    }

    /// Upright size in pixels, as shown (pending turns included); 0 before the import.
    pub(super) fn full_width(&self, item: i32) -> i32 {
        self.entry(item).map_or(0, |e| i32::try_from(e.shown_size().0).unwrap_or(i32::MAX))
    }

    pub(super) fn full_height(&self, item: i32) -> i32 {
        self.entry(item).map_or(0, |e| i32::try_from(e.shown_size().1).unwrap_or(i32::MAX))
    }

    /// Quarter turns clockwise asked for but not yet written into the file (the images are served turned).
    pub(super) fn pending_turns(&self, item: i32) -> i32 {
        self.entry(item).map_or(0, |i| i32::from(i.pending_turns.get()))
    }

    /// The item's rating, 0 for none (`ROLE_RATINGS`, the status bar's symbol).
    pub(super) fn rating(&self, item: i32) -> i32 {
        self.entry(item).and_then(|i| i.rating).map_or(0, |r| i32::from(r.get()))
    }

    /// What the grid marks on a thumbnail besides its rating (the row's `badges`): a video's length (`▶ 1:23`), the
    /// format of an animated image (`GIF`, `APNG`, `WEBP`), or nothing.
    pub(super) fn badge(&self, item: i32) -> QString {
        let Some(e) = self.entry(item) else { return QString::default() };
        QString::from(&pg_wording::badge(e.media_type(), e.duration_ms(), e.ext.key()))
    }

    /// What is left of the image without its borders, as fractions of the image as shown (pending turns included);
    /// an empty rectangle without borders found. Marked ones (`borderMarked`) have theirs.
    pub(super) fn border_box(&self, item: i32) -> QRectF {
        match self.entry(item).and_then(Entry::content) {
            Some([x, y, w, h]) => QRectF::new(x, y, w, h),
            None => QRectF::default(),
        }
    }

    /// Whether the item's borders are marked as not a border (the review).
    pub(crate) fn border_marked(&self, item: i32) -> bool {
        self.entry(item).is_some_and(|e| e.borders == Borders::Marked)
    }

    /// The mark a toggle of the item starts from: the mark it will have once the writes queued for it are made, so a
    /// second toggle before the first write lands undoes the first (`mark_target`).
    pub(crate) fn border_mark_target(&self, item: i32) -> bool {
        self.entry(item).is_some_and(|e| self.mark_target(e.id, e.borders))
    }

    /// Where the border detector's first rejected candidate at `edge` of the image as shown would have cut, as a
    /// fraction of the image from that edge; -1 for none.
    pub(super) fn border_rejected(&self, item: i32, edge: Edge) -> f64 {
        self.with_details(item, |e, d| {
            // A side's line, as the shown edge it is nearest (borders cut less than half a side) and the distance.
            let shown = |[x, y, w, _]: [f64; 4]| match (w == 0.0, x < 0.5, y < 0.5) {
                (true, true, _) => (Edge::Left, x),
                (true, false, _) => (Edge::Right, 1.0 - x),
                (false, _, true) => (Edge::Top, y),
                (false, _, false) => (Edge::Bottom, 1.0 - y),
            };
            d.border_sides
                .iter()
                .filter(|s| !s.accepted())
                .filter_map(|s| e.side_line(s.edge, s.depth).map(shown))
                .find(|(at, _)| *at == edge)
                .map_or(-1.0, |(_, d)| d)
        })
        .loaded()
        .unwrap_or(-1.0)
    }

    /// How many of the listed files have borders found (and not marked): what `saveCroppedCopies` crops.
    pub(super) fn bordered_count(&self) -> i32 {
        i32::try_from(self.grid.bordered()).unwrap_or(i32::MAX)
    }

    /// What the viewer shows the item with (`viewedKind`): its media type, a still without an item.
    pub(super) fn media_type(&self, item: i32) -> MediaType {
        self.entry(item).map_or(MediaType::Image, Entry::media_type)
    }

    /// `viewedKind`: the viewer's item's media type (`refresh_viewed`), as the bridge's enum.
    pub(crate) fn read_viewed_kind(&self) -> qobject::MediaKind {
        match self.viewed_kind {
            MediaType::Image => qobject::MediaKind::Image,
            MediaType::Animated => qobject::MediaKind::Animated,
            MediaType::Video => qobject::MediaKind::Video,
        }
    }

    /// What Qt's players must add to show the file upright. An animation's frames are turned by all of the file's
    /// orientation: its EXIF tag, the catalog rotation and the turns not yet written (the mirror and the quarter turns
    /// of `Orientation::parts`). A video's own rotation tag is applied by the player, so a video gets the catalog and
    /// pending turns alone, never a mirror.
    pub(super) fn player_turns(&self, item: i32) -> PlayerTurns {
        self.entry(item).map_or(PlayerTurns::default(), |e| {
            PlayerTurns::of(e.media_type(), e.exif_orientation, e.turns.plus(e.pending_turns))
        })
    }

    /// One line for the status bar: path, dimensions, file size, where the rating is kept if not in the file (the
    /// rating's symbol is added after it, `refresh_derived`), and the borders found.
    pub(super) fn info(&self, item: i32) -> QString {
        // No item (none selected, or the viewer closed): no line at all.
        if self.entry(item).is_none() {
            return QString::default();
        }
        // While the details are read the line says so, and a file with no details says that: neither is left blank.
        match self.with_details(item, |e, d| {
            let classes: Vec<Class> = d.border_sides.iter().filter(|s| s.accepted()).map(|s| s.class()).collect();
            let borders = match (e.borders, e.shown_cut()) {
                (Borders::Found | Borders::Marked, Some(cut)) => {
                    Some(FoundBorders { marked: e.borders == Borders::Marked, cut, classes: &classes })
                }
                _ => None,
            };
            let kept = e.rating.is_some() && !d.rating_in_file;
            let path = pg_platform::native::PATH_RULES.display(&d.path);
            QString::from(&pg_wording::info(&path, e.shown_size(), d.size, kept, borders))
        }) {
            Detail::Loaded(line) => line,
            Detail::Loading => QString::from(&pg_wording::ui("ui-loading", 0.0, "", "")),
            Detail::Unavailable => QString::from(&pg_wording::ui("ui-details-unavailable", 0.0, "", "")),
        }
    }
}
