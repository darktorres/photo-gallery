//! The state of the `YandexSearch` QObject: the bridge's backing struct (the results of the app's search, and the preview).

use cxx_qt_lib::QString;
use pg_app::yandex::Hit;
use pg_client::layout::Frame;
use pg_client::results::Results;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct YandexSearchRust {
    pub(crate) busy: bool,
    pub(crate) status: QString,
    pub(crate) other_count: i32,
    pub(crate) similar_count: i32,
    pub(crate) picked_count: i32,
    pub(crate) other_picked: i32,
    pub(crate) similar_picked: i32,
    pub(crate) preview_item: i32,
    pub(crate) preview_url: QString,
    pub(crate) preview_width: i32,
    pub(crate) preview_height: i32,
    pub(crate) preview_picked: bool,
    pub(crate) layout_width: f64,
    pub(crate) gap: f64,
    pub(crate) other_height: f64,
    pub(crate) similar_height: f64,
    /// Whether the change pump runs: it starts with the first search or save, which subscribes before it starts one.
    pub(super) pump: bool,
    /// The other sizes, then the similar images, as the latest search found them, and their picks.
    pub(super) results: Results<Hit>,
    /// Each result's frame in the panel's layout (`relayout`), by its index in `hits`.
    pub(super) frames: Vec<Frame>,
}
