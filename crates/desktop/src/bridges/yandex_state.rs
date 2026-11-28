//! The state of the `YandexSearch` QObject: the bridge's backing struct (the results of the app's search, and the preview).

use std::collections::BTreeMap;
use std::path::PathBuf;

use cxx_qt_lib::QString;
use pg_app::yandex::Hit;
use pg_client::layout::Frame;
use pg_client::results::Results;

#[derive(Default)]
pub struct YandexSearchRust {
    pub(crate) busy: bool,
    pub(crate) status: QString,
    pub(crate) other_count: i32,
    pub(crate) similar_count: i32,
    pub(crate) picked_count: i32,
    pub(crate) other_picked: i32,
    pub(crate) similar_picked: i32,
    /// The first row whose picture has not settled (loaded or failed): the tiles at or before it may load (see
    /// `YandexSearch::thumb_settled`).
    pub(crate) thumb_turn: i32,
    pub(crate) preview_item: i32,
    pub(crate) preview_url: QString,
    pub(crate) preview_prev_url: QString,
    pub(crate) preview_next_url: QString,
    pub(crate) preview_width: i32,
    pub(crate) preview_height: i32,
    pub(crate) preview_picked: bool,
    pub(crate) preview_animation_url: QString,
    pub(crate) preview_mirrored: bool,
    pub(crate) preview_turns: i32,
    pub(crate) layout_width: f64,
    pub(crate) gap: f64,
    pub(crate) other_height: f64,
    pub(crate) similar_height: f64,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pub(crate) pump: bool,
    /// The other sizes, then the similar images, as the latest search found them, and their picks.
    pub(crate) results: Results<Hit>,
    /// Each result's frame in the panel's layout (`relayout`), by its index in `results`.
    pub(crate) frames: Vec<Frame>,
    /// The photo the panel is for: the one its last search was asked for, whose results and picks it shows.
    pub(crate) photo: Option<PathBuf>,
    /// The photo the app's latest accepted search is for. The panel shows that search's results, progress and status only
    /// while it is the panel's photo (`photo == searched`): a search of another photo never lands under this one.
    pub(crate) searched: Option<PathBuf>,
    /// The picks of each photo the panel showed, by the ids of its results: kept across a close and a reopen of that
    /// photo, and put back on its results when they come in (`restore_picks`).
    pub(crate) picks: BTreeMap<PathBuf, Vec<u64>>,
}
