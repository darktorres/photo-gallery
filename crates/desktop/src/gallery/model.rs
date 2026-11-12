//! The model's plumbing: the rows Qt reads, a list arriving and shown, the layout, and the values the views
//! follow (the selection's text, the viewer's item, the revision).

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QMap, QMapPair_QString_QVariant, QModelIndex, QString,
    QStringList, QVariant,
};
use pg_client::grid::{Arrival, Changed, Plan, Shape};
use pg_client::text;
use pg_model::detection::Edge;
use pg_model::grid::Entry;
use pg_model::size::ThumbHeight;

use super::qobject;
use super::roles::{ROLE_HEIGHT, ROLE_ROW, qt_int};
use super::state::fresh_entry;

impl qobject::GalleryModel {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(at) = usize::try_from(index.row()) else { return QVariant::default() };
        match role {
            ROLE_ROW => QVariant::from(&self.row_value(at)),
            ROLE_HEIGHT => match self.grid.rows().get(at) {
                Some(row) => QVariant::from(&row.height),
                None => QVariant::default(),
            },
            _ => QVariant::default(),
        }
    }

    /// The row at `at` as one value (`ROLE_ROW`): its items (`first`, `count`), the file each item is (`ids`), and the
    /// values its cells show, one list per field with an entry per item. The items' ints are Qt's, which the item
    /// getters take.
    fn row_value(&self, at: usize) -> QMap<QMapPair_QString_QVariant> {
        let mut map = QMap::<QMapPair_QString_QVariant>::default();
        let Some(row) = self.grid.rows().get(at) else { return map };
        let (Ok(first), Ok(end)) = (i32::try_from(row.first), i32::try_from(row.first + row.count)) else {
            return map;
        };
        let capacity = usize::try_from(end - first).unwrap_or(0);
        let mut ids = Vec::with_capacity(capacity);
        let mut thumbs = Vec::with_capacity(capacity);
        let mut thumbhashes = Vec::with_capacity(capacity);
        let mut badges = Vec::with_capacity(capacity);
        let mut ratings = Vec::with_capacity(capacity);
        let mut borders_marked = Vec::with_capacity(capacity);
        let mut image_aspects = Vec::with_capacity(capacity);
        let mut aspects = Vec::with_capacity(capacity);
        let mut content_x = Vec::with_capacity(capacity);
        let mut content_y = Vec::with_capacity(capacity);
        let mut content_width = Vec::with_capacity(capacity);
        let mut content_height = Vec::with_capacity(capacity);
        for i in first..end {
            let key = self.image_key(i);
            let border = self.border_box(i);
            ids.push(self.file_id(i));
            thumbs.push(self.thumb_url(&key, self.thumb_height));
            thumbhashes.push(self.thumbhash_url(&key));
            badges.push(self.badge(i));
            ratings.push(self.rating(i));
            borders_marked.push(self.border_marked(i));
            image_aspects.push(self.image_aspect(i));
            aspects.push(self.aspect(i));
            content_x.push(border.x());
            content_y.push(border.y());
            content_width.push(border.width());
            content_height.push(border.height());
        }
        map.insert_clone(&QString::from("first"), &QVariant::from(&first));
        map.insert_clone(&QString::from("count"), &QVariant::from(&qt_int(row.count)));
        map.insert_clone(&QString::from("ids"), &QVariant::from(&ids.into_iter().collect::<QList<i64>>()));
        map.insert_clone(&QString::from("thumbUrls"), &QVariant::from(&thumbs.into_iter().collect::<QStringList>()));
        map.insert_clone(
            &QString::from("thumbhashUrls"),
            &QVariant::from(&thumbhashes.into_iter().collect::<QStringList>()),
        );
        map.insert_clone(&QString::from("badges"), &QVariant::from(&badges.into_iter().collect::<QStringList>()));
        map.insert_clone(&QString::from("ratings"), &QVariant::from(&ratings.into_iter().collect::<QList<i32>>()));
        map.insert_clone(
            &QString::from("bordersMarked"),
            &QVariant::from(&borders_marked.into_iter().collect::<QList<bool>>()),
        );
        map.insert_clone(
            &QString::from("imageAspects"),
            &QVariant::from(&image_aspects.into_iter().collect::<QList<f64>>()),
        );
        map.insert_clone(&QString::from("aspects"), &QVariant::from(&aspects.into_iter().collect::<QList<f64>>()));
        map.insert_clone(&QString::from("contentX"), &QVariant::from(&content_x.into_iter().collect::<QList<f64>>()));
        map.insert_clone(&QString::from("contentY"), &QVariant::from(&content_y.into_iter().collect::<QList<f64>>()));
        map.insert_clone(
            &QString::from("contentWidth"),
            &QVariant::from(&content_width.into_iter().collect::<QList<f64>>()),
        );
        map.insert_clone(
            &QString::from("contentHeight"),
            &QVariant::from(&content_height.into_iter().collect::<QList<f64>>()),
        );
        map
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_ROW, QByteArray::from("row"));
        roles.insert(ROLE_HEIGHT, QByteArray::from("rowHeight"));
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.grid.rows().len())
    }

    /// Show the files of `listing` (a merged folder, with or without its subfolders, that pass its query, in its
    /// order). The list arrives later (`loaded`). With `keep` (the same listing, refreshed), the selection and `anchor`
    /// stay with their files; otherwise the selection is cleared. `generation` is the controller's for this list
    /// (`Command::OpenList`): the grid shows it only while it is the newest asked for, and once it is shown the window
    /// reports it back (`APPLIED`, `Shell::list_shown`).
    pub(crate) fn open_listing(
        mut self: Pin<&mut Self>,
        listing: pg_model::view::Listing,
        keep: bool,
        generation: u64,
    ) {
        let loading = self.loading;
        self.as_mut().rust_mut().grid.request(keep, loading, generation);
        // A list held for the viewer is stale now: the one asked for is loading.
        let holding = self.grid.is_held();
        self.as_mut().set_holding(holding);
        self.as_mut().set_loading(true);
        let pg_model::view::Listing { folder, recursive, sort, query } = listing;
        let model = self.qt_thread();
        let shown = pg_platform::native::PATH_RULES.display_merged(&folder);
        crate::app::get().reading().session().grid(folder, recursive, query, sort, move |entries| {
            let entries = match entries {
                Ok(entries) => {
                    crate::app::resolved("listing-failed");
                    entries
                }
                Err(e) => {
                    let error = crate::app::error_text(&format!("{shown}: {e:#}"));
                    crate::app::post(pg_model::notice::Notice::ListingFailed { error });
                    Vec::new()
                }
            };
            // Fails only when the model is gone (the window closing): nothing left to show it in.
            let _ = model.queue(move |model| model.apply(generation, entries));
        });
    }

    pub(crate) fn set_viewing_holding(mut self: Pin<&mut Self>, value: i32) {
        if self.viewing == value {
            return;
        }
        self.as_mut().rust_mut().viewing = value;
        self.as_mut().viewing_changed();
        self.as_mut().refresh_viewed();
        if value < 0
            && let Some((generation, entries)) = self.as_mut().rust_mut().grid.release_held()
        {
            self.apply(generation, entries);
        }
    }

    /// Show a list `open_listing` asked for, unless a newer one was asked for since; while the viewer is open, when
    /// it closes.
    pub(super) fn apply(mut self: Pin<&mut Self>, generation: u64, entries: Vec<Entry>) {
        let viewing = self.viewing >= 0;
        let arrival = self.as_mut().rust_mut().grid.arrive(generation, entries, viewing, fresh_entry);
        let holding = self.grid.is_held();
        self.as_mut().set_holding(holding);
        let entries = match arrival {
            Arrival::Ready(entries) => entries,
            Arrival::Stale | Arrival::Held => return,
        };
        self.as_mut().about_to_load();
        let keep = self.grid.keep();
        let (current, anchor) = (self.file_at(self.current), self.file_at(self.anchor));
        let count = qt_int(entries.len());
        let shown = {
            let mut this = self.as_mut().rust_mut();
            this.forget_details();
            this.grid.swap_entries(entries)
        };
        // The list the window shows from now on: what its `ListShown` reports (`Shell::list_shown`).
        super::APPLIED.with(|applied| applied.set(Some(generation)));
        self.as_mut().set_count(count);
        // The same folder read again: only the rows that changed change.
        let changed = if keep { Changed::List(&shown) } else { Changed::All };
        let plan = self.as_mut().rust_mut().grid.plan(changed);
        self.as_mut().show(plan);
        // Before the turns are read again: they aren't on their way any more.
        self.as_mut().set_loading(false);
        if self.as_mut().rust_mut().grid.take_turned() {
            self.as_mut().refresh_turned();
        }
        // The selection stays with its file; if the file left the list (rated out of "Unrated"), what took its place.
        let selection = self.grid.selection_after(keep, crate::rows::row_of(self.current), current, anchor);
        self.as_mut().set_current_item(crate::rows::to_row(selection.current));
        self.as_mut().set_anchor(-1);
        self.as_mut().bump_revision();
        pg_trace::pg_stage_list_shown();
        self.loaded(crate::rows::to_row(selection.anchor));
    }

    pub(crate) fn relayout(mut self: Pin<&mut Self>, width: f64, row_height: f64, spacing: f64) {
        let plan = {
            let mut this = self.as_mut().rust_mut();
            this.grid.set_size(width, row_height, spacing);
            this.grid.plan(Changed::Nothing)
        };
        self.show(plan);
    }

    /// Puts a laid-out list in (`Grid::plan`). A reset, an insert or a remove is made around the change of the rows,
    /// as Qt wants it; then each row that changed in place is sent its changed roles, in one change per run of rows.
    /// The others keep their delegates: ListView keeps the first row in view where it is through them, where a reset
    /// takes the view to the top and fills it there.
    pub(super) fn show(mut self: Pin<&mut Self>, plan: Plan) {
        let root = QModelIndex::default();
        let updates = match plan.shape() {
            Shape::Reset => {
                self.as_mut().begin_reset_model();
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_reset_model();
                updates
            }
            Shape::Insert(rows) => {
                self.as_mut().begin_insert_rows(&root, qt_int(rows.start), qt_int(rows.end) - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_insert_rows();
                updates
            }
            Shape::Remove(rows) => {
                self.as_mut().begin_remove_rows(&root, qt_int(rows.start), qt_int(rows.end) - 1);
                let updates = self.as_mut().rust_mut().grid.commit(plan);
                self.as_mut().end_remove_rows();
                updates
            }
            Shape::Same => self.as_mut().rust_mut().grid.commit(plan),
        };
        // A row's span or items, and its height, are signalled as they changed: a span or items change gives the QML the
        // row whole (`ROLE_ROW`, with its items and their values).
        for update in updates {
            let (top, bottom) =
                (self.index(qt_int(update.rows.start), 0, &root), self.index(qt_int(update.rows.end) - 1, 0, &root));
            let mut roles = Vec::with_capacity(2);
            if update.span || update.items {
                roles.push(ROLE_ROW);
            }
            if update.height {
                roles.push(ROLE_HEIGHT);
            }
            if !roles.is_empty() {
                self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
            }
        }
    }

    /// The thumbnails' height changed: each row gets a new version, and its row (its cells' URLs are made for the new
    /// height) is signalled once.
    pub(crate) fn set_thumb_height(mut self: Pin<&mut Self>, height: ThumbHeight) {
        if Some(height) == self.thumb_height {
            return;
        }
        self.as_mut().rust_mut().thumb_height = Some(height);
        let rows = self.as_mut().rust_mut().grid.touch_all();
        for row in 0..rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_ROW][..]));
        }
    }

    /// Items changed (rated, turned, marked) without changing shape: their rows are signalled once, with their new
    /// version (`ROLE_ROW`).
    pub(super) fn items_changed(mut self: Pin<&mut Self>, items: &[usize]) {
        let rows = self.as_mut().rust_mut().grid.touch(items);
        for row in rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_ROW][..]));
        }
    }

    /// The selection (`current`) changed, from QML or from a new list: its text is read again.
    pub(crate) fn set_current_item(mut self: Pin<&mut Self>, value: i32) {
        if value == self.current {
            return;
        }
        self.as_mut().rust_mut().current = value;
        self.as_mut().current_changed();
        self.as_mut().refresh_derived();
    }

    /// The grid's values changed (the list, a turn, a rating, a border mark): what the views read from them is read
    /// again first, then the revision moves on, which the views follow.
    pub(super) fn bump_revision(mut self: Pin<&mut Self>) {
        self.as_mut().refresh_viewed();
        self.as_mut().refresh_derived();
        let revision = self.revision + 1;
        self.as_mut().set_revision(revision);
    }

    /// The status bar's selection text and the bordered count, the values the views used to call for.
    pub(super) fn refresh_derived(mut self: Pin<&mut Self>) {
        let selection = if self.current >= 0 {
            let rating = self.rating(self.current);
            let symbol = if rating > 0 {
                text::rating_symbol(u8::try_from(rating.clamp(0, 5)).unwrap_or(0)).to_string()
            } else {
                String::new()
            };
            format!("{}   {symbol}", self.info(self.current))
        } else {
            String::new()
        };
        self.as_mut().set_selection_info(QString::from(&selection));
        let bordered = self.bordered_count();
        self.as_mut().set_croppable(bordered);
        super::publish_shown(self.shown_file());
    }

    /// Sets what the viewer shows of its item (`viewing`), the viewed properties. Read again wherever what they show
    /// may have changed: when the item is set (`set_viewing_holding`), and before each change of `revision`. With no
    /// item (`-1`, the viewer closed) each reads as the item-less value (`""`, `0`, an empty rectangle, `-1` for a
    /// border line), which is what the viewer shows nothing with.
    pub(super) fn refresh_viewed(mut self: Pin<&mut Self>) {
        super::publish_shown(self.shown_file());
        let item = self.viewing;
        let key = self.image_key(item);
        let full = self.full_url(&key);
        self.as_mut().set_viewed_key(key);
        self.as_mut().set_viewed_full_url(full);
        let neighbours = self.keys_around(item, 4);
        self.as_mut().set_viewed_neighbour_keys(neighbours);
        let turns = self.pending_turns(item);
        self.as_mut().set_viewed_turns(turns);
        let rating = self.rating(item);
        self.as_mut().set_viewed_rating(rating);
        let width = self.full_width(item);
        self.as_mut().set_viewed_full_width(width);
        let height = self.full_height(item);
        self.as_mut().set_viewed_full_height(height);
        let kind = self.media_type(item);
        if self.viewed_kind != kind {
            self.as_mut().rust_mut().viewed_kind = kind;
            self.as_mut().viewed_kind_changed();
        }
        let player_turns = self.player_turns(item);
        self.as_mut().set_viewed_player_turns(player_turns);
        let file_id = self.file_id(item);
        self.as_mut().set_viewed_file_id(file_id);
        let info = self.info(item);
        self.as_mut().set_viewed_info(info);
        let border_box = self.border_box(item);
        self.as_mut().set_viewed_border_box(border_box);
        let top = self.border_rejected(item, Edge::Top);
        self.as_mut().set_viewed_rejected_top(top);
        let bottom = self.border_rejected(item, Edge::Bottom);
        self.as_mut().set_viewed_rejected_bottom(bottom);
        let left = self.border_rejected(item, Edge::Left);
        self.as_mut().set_viewed_rejected_left(left);
        let right = self.border_rejected(item, Edge::Right);
        self.as_mut().set_viewed_rejected_right(right);
        let marked = self.border_marked(item);
        self.as_mut().set_viewed_border_marked(marked);
    }
}
