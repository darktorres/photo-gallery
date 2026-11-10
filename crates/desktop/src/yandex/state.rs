//! The state of the `YandexSearch` QObject: the bridge's backing struct (the search, its results and the preview).

use cxx_qt_lib::QString;
use pg_app::yandex::{Hit, Search};
use pg_client::layout::Frame;
use pg_client::picks::Picks;

#[derive(Default)]
pub struct YandexSearchRust {
    pub(super) busy: bool,
    pub(super) status: QString,
    pub(super) other_count: i32,
    pub(super) similar_count: i32,
    pub(super) picked_count: i32,
    pub(super) other_picked: i32,
    pub(super) similar_picked: i32,
    pub(super) preview_item: i32,
    pub(super) preview_url: QString,
    pub(super) preview_width: i32,
    pub(super) preview_height: i32,
    pub(super) preview_picked: bool,
    pub(super) layout_width: f64,
    pub(super) gap: f64,
    pub(super) other_height: f64,
    pub(super) similar_height: f64,
    pub(super) search: Search,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pub(super) pump: bool,
    /// The other sizes, then the similar images, as the latest search found them.
    pub(super) hits: Vec<Hit>,
    /// Each result's frame in the panel's layout (`relayout`), by its index in `hits`.
    pub(super) frames: Vec<Frame>,
    pub(super) picks: Picks,
}
