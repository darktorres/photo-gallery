//! The model's plumbing: the rows Qt reads, a list arriving and shown, the layout, and the values the views
//! follow (the selection's text, the viewer's item, the revision).

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QPointF, QString, QStringList, QVariant};
use pg_client::grid::{Arrival, Changed, Plan, Shape};
use pg_client::text;
use pg_model::grid::Entry;

use super::qobject;
use super::roles::{
    CELL_ROLES, ROLE_ASPECTS, ROLE_BADGES, ROLE_BORDERS_MARKED, ROLE_CONTENT_HEIGHT, ROLE_CONTENT_WIDTH, ROLE_CONTENT_X,
    ROLE_CONTENT_Y, ROLE_HEIGHT, ROLE_IMAGE_ASPECTS, ROLE_RATINGS, ROLE_SPAN, ROLE_THUMB_URLS, ROLE_THUMBHASH_URLS,
    ROLE_VERSION, cell_roles, qt_int,
};
use super::state::fresh_entry;

impl qobject::GalleryModel {
    pub(super) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(at) = usize::try_from(index.row()) else { return QVariant::default() };
        let Some(row) = self.grid.rows().get(at) else { return QVariant::default() };
        // The row's items as Qt's ints, which the item getters take.
        let (Ok(first), Ok(end)) = (i32::try_from(row.first), i32::try_from(row.first + row.count)) else {
            return QVariant::default();
        };
        let items = first..end;
        match role {
            ROLE_SPAN => {
                QVariant::from(&QPointF::new(pg_model::num::count(row.first), pg_model::num::count(row.count)))
            }
            ROLE_HEIGHT => QVariant::from(&row.height),
            // The version's bits are the tag Qt compares: as the catalog's counter, it wraps where it wraps.
            ROLE_VERSION => QVariant::from(&(self.grid.version(at).cast_signed())),
            ROLE_THUMB_URLS => QVariant::from(
                &items.map(|i| self.thumb_url(&self.image_key(i), self.thumb_height)).collect::<QStringList>(),
            ),
            ROLE_THUMBHASH_URLS => {
                QVariant::from(&items.map(|i| self.thumbhash_url(&self.image_key(i))).collect::<QStringList>())
            }
            ROLE_BADGES => QVariant::from(&items.map(|i| self.badge(i)).collect::<QStringList>()),
            ROLE_RATINGS => QVariant::from(&items.map(|i| self.rating(i)).collect::<QList<i32>>()),
            ROLE_BORDERS_MARKED => QVariant::from(&items.map(|i| self.border_marked(i)).collect::<QList<bool>>()),
            ROLE_IMAGE_ASPECTS => QVariant::from(&items.map(|i| self.image_aspect(i)).collect::<QList<f64>>()),
            ROLE_ASPECTS => QVariant::from(&items.map(|i| self.aspect(i)).collect::<QList<f64>>()),
            ROLE_CONTENT_X => QVariant::from(&items.map(|i| self.border_box(i).x()).collect::<QList<f64>>()),
            ROLE_CONTENT_Y => QVariant::from(&items.map(|i| self.border_box(i).y()).collect::<QList<f64>>()),
            ROLE_CONTENT_WIDTH => QVariant::from(&items.map(|i| self.border_box(i).width()).collect::<QList<f64>>()),
            ROLE_CONTENT_HEIGHT => QVariant::from(&items.map(|i| self.border_box(i).height()).collect::<QList<f64>>()),
            _ => QVariant::default(),
        }
    }

    pub(super) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_SPAN, QByteArray::from("span"));
        roles.insert(ROLE_HEIGHT, QByteArray::from("rowHeight"));
        roles.insert(ROLE_VERSION, QByteArray::from("version"));
        for (role, name) in CELL_ROLES {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(super) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.grid.rows().len())
    }

    /// Show the files of `listing` (a merged folder, with or without its subfolders, that pass its query, in its
    /// order). The list arrives later (`loaded`). With `keep` (the same listing, refreshed), the selection and `anchor`
    /// stay with their files; otherwise the selection is cleared.
    pub(crate) fn open_listing(mut self: Pin<&mut Self>, listing: pg_model::view::Listing, keep: bool) {
        let loading = self.loading;
        let generation = self.as_mut().rust_mut().grid.request(keep, loading);
        // A list held for the viewer is stale now: the one asked for is loading.
        let holding = self.grid.is_held();
        self.as_mut().set_holding(holding);
        self.as_mut().set_loading(true);
        let pg_model::view::Listing { folder, recursive, sort, query } = listing;
        let model = self.qt_thread();
        let shown = pg_model::rules::PathRules::NATIVE.display_merged(&folder);
        crate::app::get().reading().session().grid(folder, recursive, query, sort, move |entries| {
            let entries = entries.unwrap_or_else(|e| {
                crate::log::note(&format!("listing {shown}: {e:#}"));
                Vec::new()
            });
            // Fails only when the model is gone (the window closing): nothing left to show it in.
            let _ = model.queue(move |model| model.apply(generation, entries));
        });
    }

    pub(super) fn set_viewing_holding(mut self: Pin<&mut Self>, value: i32) {
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
        let (current, anchor) = (self.grid.id_at(self.current), self.grid.id_at(self.anchor));
        let count = qt_int(entries.len());
        let shown = {
            let mut this = self.as_mut().rust_mut();
            this.forget_details();
            this.grid.swap_entries(entries)
        };
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
        let selection = self.grid.selection_after(keep, self.current, current, anchor);
        self.as_mut().set_current_item(selection.current);
        self.as_mut().set_anchor(-1);
        self.as_mut().bump_revision();
        pg_trace::pg_stage_list_shown();
        self.loaded(selection.anchor);
    }

    pub(super) fn relayout(mut self: Pin<&mut Self>, width: f64, row_height: f64, spacing: f64) {
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
        // A row's new span and its new version can come in one change: the QML reads its items again either way, so
        // every role that changed is signalled. The cells' lists go first, in a change of their own: the row takes
        // its values when its span or version arrives, and they must all be in by then.
        for update in updates {
            let (top, bottom) =
                (self.index(qt_int(update.rows.start), 0, &root), self.index(qt_int(update.rows.end) - 1, 0, &root));
            if update.span || update.version {
                self.as_mut().data_changed(&top, &bottom, &QList::from(cell_roles().as_slice()));
            }
            let mut roles = Vec::with_capacity(3);
            if update.span {
                roles.push(ROLE_SPAN);
            }
            if update.version {
                roles.push(ROLE_VERSION);
            }
            if update.height {
                roles.push(ROLE_HEIGHT);
            }
            self.as_mut().data_changed(&top, &bottom, &QList::from(roles.as_slice()));
        }
    }

    /// The thumbnails' height changed: each row gets a new version and its cells' lists are read again (the URLs are
    /// made for the new height), in the same order as `items_changed`.
    pub(super) fn set_thumb_height(mut self: Pin<&mut Self>, height: i32) {
        if height == self.thumb_height {
            return;
        }
        self.as_mut().rust_mut().thumb_height = height;
        let rows = self.as_mut().rust_mut().grid.touch_all();
        for row in 0..rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(cell_roles().as_slice()));
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
        }
    }

    /// Items changed (rated, turned, marked) without changing shape: the cells of their rows read them again (the
    /// cells' lists, then `ROLE_VERSION`).
    pub(super) fn items_changed(mut self: Pin<&mut Self>, items: &[usize]) {
        let rows = self.as_mut().rust_mut().grid.touch(items);
        for row in rows {
            let index = self.index(qt_int(row), 0, &QModelIndex::default());
            self.as_mut().data_changed(&index, &index, &QList::from(cell_roles().as_slice()));
            self.as_mut().data_changed(&index, &index, &QList::from(&[ROLE_VERSION][..]));
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
    }

    /// Sets what the viewer shows of its item (`viewing`), the viewed properties. Read again wherever what they show
    /// may have changed: when the item is set (`set_viewing_holding`), and before each change of `revision`. With no
    /// item (`-1`, the viewer closed) each reads as the item-less value (`""`, `0`, an empty rectangle, `-1` for a
    /// border line), which is what the viewer shows nothing with.
    pub(super) fn refresh_viewed(mut self: Pin<&mut Self>) {
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
        let top = self.border_rejected(item, "top");
        self.as_mut().set_viewed_rejected_top(top);
        let bottom = self.border_rejected(item, "bottom");
        self.as_mut().set_viewed_rejected_bottom(bottom);
        let left = self.border_rejected(item, "left");
        self.as_mut().set_viewed_rejected_left(left);
        let right = self.border_rejected(item, "right");
        self.as_mut().set_viewed_rejected_right(right);
        let marked = self.border_marked(item);
        self.as_mut().set_viewed_border_marked(marked);
    }
}
