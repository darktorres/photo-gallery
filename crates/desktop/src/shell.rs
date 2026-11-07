//! The window's shell: the one object the window reports its events to. Each event goes to the window's controller
//! (`controller.rs`, `pg_client::controller`), which decides what happens; the shell carries out the commands that come
//! back. The model's commands (the selection, the steps, ratings, turns, lists, border marks) go to the grid model on its
//! own lane (`gallery::post`); its signals are what Main.qml presents (the grid's scroll, the viewer, the list shown,
//! the layout); the session is saved here, and the properties it mirrors
//! (the folder, the sort, the panels, the sizes) are copied from the controller after each event. The one deadline timer
//! (`deadline.rs`) is armed to the controller's next pause.
//!
//! Re-entrance: a signal is emitted within the call that caused it, and so is a qproperty's notify. A QML handler of one
//! of these signals or notifies must not call the shell back synchronously; the signals that do call it are emitted
//! later (`later`), once the call has returned.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The merged folder shown (`""` = all folders). Setting it only spells it (the tree's spelling, which the
        /// controller remembers); to show a folder, `openFolder`.
        #[qproperty(QString, folder, READ, WRITE = set_folder, NOTIFY, FINAL)]
        /// With the files of its subfolders (`setRecursive`).
        #[qproperty(bool, recursive, READ, NOTIFY, FINAL)]
        /// Which way the files are ordered (the key is the sort box's, `sortIndex`); `toggleDescending` turns it.
        #[qproperty(bool, descending, READ, NOTIFY, FINAL)]
        /// Names of the saved views, sorted.
        #[qproperty(QStringList, saved_views, cxx_name = "savedViews", READ, WRITE, NOTIFY, FINAL)]
        /// The filter bar is open (`Shell.setFiltersOpen`, `Shell.toggleFilters`).
        #[qproperty(bool, filters_open, cxx_name = "filtersOpen", READ, NOTIFY, FINAL)]
        /// The borders are shown (`Shell.setShowBorders`, `Shell.toggleBorders`).
        #[qproperty(bool, show_borders, cxx_name = "showBorders", READ, NOTIFY, FINAL)]
        /// The row height (`Shell.sizeChosen`, `Shell.sizeStepped`, the grid's keys).
        #[qproperty(i32, row_height, cxx_name = "rowHeight", READ, NOTIFY, FINAL)]
        /// The thumbnail height: the row height times the screen's scale, settled after a pause.
        #[qproperty(i32, thumb_height, cxx_name = "thumbHeight", READ, NOTIFY, FINAL)]
        type Shell = super::ShellRust;
    }

    impl cxx_qt::Threading for Shell {}

    unsafe extern "RustQt" {
        /// The folder tree was loaded. `folderKnown`: the listed folder is in it (or it is all folders, "").
        #[qinvokable]
        #[cxx_name = "treeLoaded"]
        fn tree_loaded(self: Pin<&mut Shell>, folder_known: bool);

        /// A list of `count` items was shown. `lastFile`: the index in it of the file the last session left, -1 when none.
        #[qinvokable]
        #[cxx_name = "listShown"]
        fn list_shown(self: Pin<&mut Shell>, last_file: i32, count: i32);

        /// The list shown was placed in the view (its rows are positioned).
        #[qinvokable]
        #[cxx_name = "listPositioned"]
        fn list_positioned(self: Pin<&mut Shell>);

        /// The start is over: the test hooks run and the background job starts.
        #[qsignal]
        fn started(self: Pin<&mut Shell>);

        /// The current item changed (-1: none).
        #[qinvokable]
        #[cxx_name = "currentChanged"]
        fn current_changed(self: Pin<&mut Shell>, index: i32);

        /// A grid key was pressed (Qt's key code and modifiers). True when the grid uses it: the controller acts on it
        /// (`selectRequested`, the model, `viewerRequested`), or changes the size or a panel.
        #[qinvokable]
        #[cxx_name = "keyPressed"]
        fn key_pressed(self: Pin<&mut Shell>, key: i32, modifiers: i32) -> bool;

        /// The item a viewer step from `from` by `d` lands on, clamped to the list (the viewer moves there).
        #[qinvokable]
        #[cxx_name = "viewerStep"]
        fn viewer_step(self: &Shell, from: i32, d: i32) -> i32;

        /// The grid's width changed (in pixels): a relayout is due after a short pause (`relayoutDue`).
        #[qinvokable]
        #[cxx_name = "gridResized"]
        fn grid_resized(self: Pin<&mut Shell>, width: f64);

        /// The filter bar's query was edited: the list is shown after a pause (`openList` by the controller).
        #[qinvokable]
        #[cxx_name = "queryEdited"]
        fn query_edited(self: Pin<&mut Shell>);

        /// The size chosen (the size box, the session's saved height).
        #[qinvokable]
        #[cxx_name = "sizeChosen"]
        fn size_chosen(self: Pin<&mut Shell>, height: f64);

        /// The row height multiplied by `factor` (a pinch, a wheel step).
        #[qinvokable]
        #[cxx_name = "sizeStepped"]
        fn size_stepped(self: Pin<&mut Shell>, factor: f64);

        /// The screen's scale (device pixel ratio).
        #[qinvokable]
        #[cxx_name = "setScale"]
        fn set_scale(self: Pin<&mut Shell>, scale: f64);

        #[qinvokable]
        #[cxx_name = "toggleFilters"]
        fn toggle_filters(self: Pin<&mut Shell>);

        #[qinvokable]
        #[cxx_name = "setFiltersOpen"]
        fn set_filters_open(self: Pin<&mut Shell>, on: bool);

        #[qinvokable]
        #[cxx_name = "toggleBorders"]
        fn toggle_borders(self: Pin<&mut Shell>);

        #[qinvokable]
        #[cxx_name = "setShowBorders"]
        fn set_show_borders(self: Pin<&mut Shell>, on: bool);

        /// The viewer is open or shut: while it is open, the grid does not take the keys (`keyPressed` is false).
        #[qinvokable]
        #[cxx_name = "setViewerOpen"]
        fn set_viewer_open(self: Pin<&mut Shell>, on: bool);

        /// Whether a pause is still running (the screenshot waits for none).
        #[qinvokable]
        #[cxx_name = "pending"]
        fn pending(self: &Shell) -> bool;

        /// Shutdown: the pending saves run now (the session and the row height are kept).
        #[qinvokable]
        #[cxx_name = "flush"]
        fn flush(self: Pin<&mut Shell>);

        /// A rating, asked for on an item (the viewer's keys and buttons).
        #[qinvokable]
        #[cxx_name = "rate"]
        fn rate(self: Pin<&mut Shell>, item: i32, rating: i32);

        /// A quarter turn of an item (the viewer's keys and buttons): 1 clockwise, -1 counterclockwise.
        #[qinvokable]
        #[cxx_name = "turn"]
        fn turn(self: Pin<&mut Shell>, item: i32, turns: i32);

        /// Toggle an item's border mark (the test hooks and the grid's key).
        #[qinvokable]
        #[cxx_name = "toggleBorderMark"]
        fn toggle_border_mark(self: Pin<&mut Shell>, item: i32);

        /// Set an item's border mark (the viewer's button).
        #[qinvokable]
        #[cxx_name = "setBorderMark"]
        fn set_border_mark(self: Pin<&mut Shell>, item: i32, marked: bool);

        /// Files were turned in place on disk: their rows are read again (nothing is renumbered).
        #[qinvokable]
        #[cxx_name = "filesChanged"]
        fn files_changed(self: Pin<&mut Shell>);

        /// The list is read again (new files, new sizes), the selection kept.
        #[qinvokable]
        #[cxx_name = "reloadList"]
        fn reload_list(self: Pin<&mut Shell>);

        /// Scroll this item into view: it is the selection already (the model set it).
        #[qsignal]
        #[cxx_name = "selectRequested"]
        fn select_requested(self: Pin<&mut Shell>, index: i32);

        /// Show the viewer at this item.
        #[qsignal]
        #[cxx_name = "viewerRequested"]
        fn viewer_requested(self: Pin<&mut Shell>, index: i32);

        /// The selection rested: remember the file it is on.
        #[qsignal]
        #[cxx_name = "sessionDue"]
        fn session_due(self: Pin<&mut Shell>);

        /// The grid's width or row height settled: lay the grid out again.
        #[qsignal]
        #[cxx_name = "relayoutDue"]
        fn relayout_due(self: Pin<&mut Shell>);

        /// Remember this row height for the next start.
        #[qsignal]
        #[cxx_name = "saveRowHeight"]
        fn save_row_height(self: Pin<&mut Shell>, height: i32);

        /// A list was asked for (the folder, subfolders, sort, query or saved view changed, or the files were read
        /// again): the facets and the sort box follow it.
        #[qsignal]
        #[cxx_name = "listingChanged"]
        fn listing_changed(self: Pin<&mut Shell>);

        fn set_folder(self: Pin<&mut Shell>, value: &QString);

        /// Show a merged folder (`""` = all folders), with the current subfolders, sort and query.
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: Pin<&mut Shell>, folder: &QString);

        #[qinvokable]
        #[cxx_name = "setRecursive"]
        fn set_recursive(self: Pin<&mut Shell>, on: bool);

        #[qinvokable]
        #[cxx_name = "toggleDescending"]
        fn toggle_descending(self: Pin<&mut Shell>);

        /// Remember the listing (folder, subfolders, sort and query) for the next start.
        #[qinvokable]
        #[cxx_name = "rememberListing"]
        fn remember_listing(self: Pin<&mut Shell>);

        /// Save (or replace) the listing under a name: its folder, subfolders, sort and query.
        #[qinvokable]
        #[cxx_name = "saveView"]
        fn save_view(self: Pin<&mut Shell>, name: &QString);

        #[qinvokable]
        #[cxx_name = "deleteView"]
        fn delete_view(self: Pin<&mut Shell>, name: &QString);

        /// Show a saved view: its folder, subfolders, sort and query replace the listing's. `false` if there is no view
        /// by that name. The filter bar then reloads the query (`FilterBar.reloadQuery`).
        #[qinvokable]
        #[cxx_name = "applyView"]
        fn apply_view(self: Pin<&mut Shell>, name: &QString) -> bool;

        /// Order by the sort box's choice `index` (`pg_client::sort`): its key, and its way.
        #[qinvokable]
        #[cxx_name = "sortBy"]
        fn sort_by(self: Pin<&mut Shell>, index: i32);

        /// The sort box's index for the key the files are ordered by.
        #[qinvokable]
        #[cxx_name = "sortIndex"]
        fn sort_index(self: &Shell) -> i32;
    }
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_client::controller::listing::Event as ListingEvent;
use pg_client::controller::panels;
use pg_client::controller::prefs;
use pg_client::controller::{Command, Event};
use pg_client::sort::choice_index;
use pg_model::view::SavedView;

use crate::{app, controller, deadline, gallery, keymap};

/// The properties the shell mirrors from the controller. Everything else is the controller's.
pub struct ShellRust {
    folder: QString,
    recursive: bool,
    descending: bool,
    saved_views: QStringList,
    filters_open: bool,
    show_borders: bool,
    row_height: i32,
    thumb_height: i32,
}

impl Default for ShellRust {
    /// The controller's start, mirrored: the listing the window starts with, and the size's defaults (the window sets
    /// the size and the scale when it is made).
    fn default() -> ShellRust {
        let listing = controller::listing();
        let prefs = controller::with(|c| *c.prefs());
        ShellRust {
            folder: QString::from(&listing.folder),
            recursive: listing.recursive,
            descending: listing.sort.descending,
            saved_views: view_names(),
            filters_open: false,
            show_borders: false,
            row_height: prefs.row_height(),
            thumb_height: prefs.thumb_height(),
        }
    }
}

/// A list index as QML takes it (-1 for none).
fn to_i32(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(-1)
}

/// The names of the saved views, sorted (`saved_views`).
fn view_names() -> QStringList {
    app::get()
        .views()
        .unwrap_or_else(|e| {
            crate::panic_log::note(&format!("views: {e:#}"));
            Vec::new()
        })
        .iter()
        .map(|v| QString::from(&v.name))
        .collect()
}

impl qobject::Shell {
    fn tree_loaded(self: Pin<&mut Self>, folder_known: bool) {
        self.dispatch(Event::TreeLoaded { folder_known });
    }

    fn list_shown(self: Pin<&mut Self>, last_file: i32, count: i32) {
        let count = usize::try_from(count).unwrap_or(0);
        let last_file = usize::try_from(last_file).ok();
        self.dispatch(Event::ListShown { count, last_file });
    }

    fn list_positioned(self: Pin<&mut Self>) {
        self.dispatch(Event::ListPositioned);
    }

    fn current_changed(self: Pin<&mut Self>, index: i32) {
        self.dispatch(Event::CurrentChanged(usize::try_from(index).ok()));
    }

    fn key_pressed(self: Pin<&mut Self>, key: i32, modifiers: i32) -> bool {
        // While the viewer is open, its keys are its own: the grid leaves them.
        if controller::with(|c| c.panels().viewer_open()) {
            return false;
        }
        let has_current = controller::with(|c| c.has_current());
        let Some(grid) = keymap::grid_key(key, modifiers, has_current) else {
            return false;
        };
        self.dispatch(Event::Key(grid));
        true
    }

    fn viewer_step(&self, from: i32, d: i32) -> i32 {
        let from = usize::try_from(from).unwrap_or(0);
        to_i32(controller::with(|c| c.viewer_step(from, d)))
    }

    fn grid_resized(self: Pin<&mut Self>, width: f64) {
        self.dispatch(Event::GridResized(width));
    }

    fn query_edited(self: Pin<&mut Self>) {
        self.dispatch(Event::QueryEdited);
    }

    fn size_chosen(self: Pin<&mut Self>, height: f64) {
        self.dispatch(Event::Size(prefs::Event::Chosen(height)));
    }

    fn size_stepped(self: Pin<&mut Self>, factor: f64) {
        self.dispatch(Event::Size(prefs::Event::Factor(factor)));
    }

    fn set_scale(self: Pin<&mut Self>, scale: f64) {
        self.dispatch(Event::Size(prefs::Event::Scale(scale)));
    }

    fn toggle_filters(self: Pin<&mut Self>) {
        self.dispatch(Event::Panel(panels::Event::ToggleFilters));
    }

    fn set_filters_open(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Panel(panels::Event::SetFiltersOpen(on)));
    }

    fn toggle_borders(self: Pin<&mut Self>) {
        self.dispatch(Event::Panel(panels::Event::ToggleBorders));
    }

    fn set_show_borders(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Panel(panels::Event::SetShowBorders(on)));
    }

    fn set_viewer_open(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Panel(panels::Event::SetViewerOpen(on)));
    }

    fn pending(&self) -> bool {
        controller::with(|c| c.pending())
    }

    /// Shutdown: the saves that wait run now; a pause that would only show the list does not.
    fn flush(self: Pin<&mut Self>) {
        self.dispatch(Event::Flush);
    }

    fn rate(self: Pin<&mut Self>, item: i32, rating: i32) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::Rate { item, rating });
        }
    }

    fn turn(self: Pin<&mut Self>, item: i32, turns: i32) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::Turn { item, turns });
        }
    }

    fn toggle_border_mark(self: Pin<&mut Self>, item: i32) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::ToggleBorderMark(item));
        }
    }

    fn set_border_mark(self: Pin<&mut Self>, item: i32, marked: bool) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::SetBorderMark { item, marked });
        }
    }

    fn files_changed(self: Pin<&mut Self>) {
        self.dispatch(Event::FilesChanged);
    }

    fn reload_list(self: Pin<&mut Self>) {
        self.dispatch(Event::Reload);
    }

    /// The timer fired (queued from `deadline.rs`): the pauses that are due run.
    pub(crate) fn timeout(self: Pin<&mut Self>) {
        self.dispatch(Event::Timeout);
    }

    fn set_folder(self: Pin<&mut Self>, value: &QString) {
        self.dispatch(Event::FolderSpelled(value.to_string()));
    }

    fn open_folder(self: Pin<&mut Self>, folder: &QString) {
        self.dispatch(Event::Listing(ListingEvent::OpenFolder(folder.to_string())));
    }

    fn set_recursive(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Listing(ListingEvent::Recursive(on)));
    }

    fn toggle_descending(self: Pin<&mut Self>) {
        self.dispatch(Event::Listing(ListingEvent::DescendingToggled));
    }

    fn sort_by(self: Pin<&mut Self>, index: i32) {
        self.dispatch(Event::Listing(ListingEvent::SortChoice(index)));
    }

    fn sort_index(&self) -> i32 {
        choice_index(controller::listing().sort.key)
    }

    fn remember_listing(self: Pin<&mut Self>) {
        self.dispatch(Event::RememberListing);
    }

    fn save_view(mut self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let listing = controller::listing();
        let view = SavedView {
            name,
            folder: listing.folder,
            recursive: listing.recursive,
            query: listing.query,
            sort: listing.sort,
        };
        if let Err(e) = app::get().save_view(&view) {
            crate::panic_log::note(&format!("saving view: {e:#}"));
        }
        self.as_mut().set_saved_views(view_names());
    }

    fn delete_view(mut self: Pin<&mut Self>, name: &QString) {
        if let Err(e) = app::get().delete_view(&name.to_string()) {
            crate::panic_log::note(&format!("deleting view: {e:#}"));
        }
        self.as_mut().set_saved_views(view_names());
    }

    fn apply_view(self: Pin<&mut Self>, name: &QString) -> bool {
        let Some(v) = app::get().view(&name.to_string()).ok().flatten() else {
            return false;
        };
        self.dispatch(Event::Listing(ListingEvent::ViewApplied(v)));
        true
    }

    /// Report `event` to the controller, then carry out what it gives: the mirrored properties follow the listing, the
    /// one timer is set to the next pause, and each command runs in order.
    fn dispatch(mut self: Pin<&mut Self>, event: Event) {
        let commands = controller::update(event);
        self.as_mut().mirror();
        self.as_mut().rearm();
        for command in commands {
            self.as_mut().execute(command);
        }
    }

    /// Copy the listing into the properties, notifying those that changed.
    fn mirror(mut self: Pin<&mut Self>) {
        let listing = controller::listing();
        let folder = QString::from(&listing.folder);
        if self.folder != folder {
            self.as_mut().rust_mut().folder = folder;
            self.as_mut().folder_changed();
        }
        if self.recursive != listing.recursive {
            self.as_mut().rust_mut().recursive = listing.recursive;
            self.as_mut().recursive_changed();
        }
        if self.descending != listing.sort.descending {
            self.as_mut().rust_mut().descending = listing.sort.descending;
            self.as_mut().descending_changed();
        }
    }

    /// Carry out one of the controller's commands.
    fn execute(mut self: Pin<&mut Self>, command: Command) {
        match command {
            // The selection is the model's, set on the model's lane. Select then asks the grid to show the item;
            // RestoreSelection only sets it (the grid is placed on the list when it is shown, `listShown`).
            Command::Select(i) => {
                let item = to_i32(i);
                gallery::post(move |model| {
                    if model.is_item(item) {
                        model.set_current_item(item);
                    }
                });
                self.as_mut().later(move |shell| shell.select_requested(item));
            }
            Command::RestoreSelection(i) => {
                let item = to_i32(i);
                gallery::post(move |model| {
                    if model.is_item(item) {
                        model.set_current_item(item);
                    }
                });
            }
            Command::Started => self.as_mut().later(|shell| shell.started()),
            // The neighbour is the grid's (its layout is the model's). It is found and set on the model's lane, so a
            // step queued behind this one starts from it; only the scroll goes back to the shell.
            Command::Step { dx, dy } => {
                let shell = self.qt_thread();
                gallery::post(move |mut model| {
                    let item = model.neighbor_of_selection(dx, dy);
                    if model.is_item(item) {
                        model.as_mut().set_current_item(item);
                        let _ = shell.queue(move |s| s.select_requested(item));
                    }
                });
            }
            Command::ShowViewer(i) => {
                let i = to_i32(i);
                self.as_mut().later(move |shell| shell.viewer_requested(i));
            }
            Command::OpenList { listing, keep } => {
                gallery::post(move |model| model.open_listing(listing, keep));
                self.as_mut().later(|shell| shell.listing_changed());
            }
            Command::Rate { item, rating } => {
                let item = to_i32(item);
                gallery::post(move |model| model.set_rating(item, rating));
            }
            Command::Turn { item, turns } => {
                let item = to_i32(item);
                gallery::post(move |model| model.rotate(item, turns));
            }
            Command::ToggleBorderMark(item) => {
                let item = to_i32(item);
                gallery::post(move |model| {
                    let marked = !model.border_marked(item);
                    model.set_border_marked(item, marked);
                });
            }
            Command::SetBorderMark { item, marked } => {
                let item = to_i32(item);
                gallery::post(move |model| model.set_border_marked(item, marked));
            }
            Command::RefreshTurned => gallery::post(|model| model.refresh_turned()),
            Command::SaveSession(listing) => {
                if let Err(e) = app::get().save_listing(&listing) {
                    crate::panic_log::note(&format!("session: {e:#}"));
                }
            }
            // Synchronous: the save must run at shutdown too (`flush`), and its handler calls no shell.
            Command::SaveLastFile => self.as_mut().session_due(),
            Command::Relayout { .. } => self.as_mut().later(|shell| shell.relayout_due()),
            Command::Panel(change) => self.as_mut().panel(change),
            Command::Size(change) => self.as_mut().size(change),
        }
    }

    /// A panel's flag, mirrored (the viewer's flag is the controller's alone: `key_pressed` reads it).
    fn panel(mut self: Pin<&mut Self>, change: panels::Change) {
        match change {
            panels::Change::FiltersOpen(on) => {
                self.as_mut().rust_mut().filters_open = on;
                self.as_mut().filters_open_changed();
            }
            panels::Change::ShowBorders(on) => {
                self.as_mut().rust_mut().show_borders = on;
                self.as_mut().show_borders_changed();
            }
            panels::Change::ViewerOpen(_) => {}
        }
    }

    /// A size's change, mirrored; the saved row height is QML's to store (`Shell.saveRowHeight`).
    fn size(mut self: Pin<&mut Self>, change: prefs::Change) {
        match change {
            prefs::Change::RowHeight(height) => {
                self.as_mut().rust_mut().row_height = height;
                self.as_mut().row_height_changed();
            }
            prefs::Change::ThumbHeight(height) => {
                self.as_mut().rust_mut().thumb_height = height;
                self.as_mut().thumb_height_changed();
            }
            prefs::Change::SaveRowHeight(height) => self.as_mut().save_row_height(height),
        }
    }

    /// Emit a signal after this call. Its handler in Main.qml calls back into the shell (the list, the layout, the
    /// selection), which it cannot do inside the call that emitted the signal; so the emission waits for the event loop.
    fn later(self: Pin<&mut Self>, emit: impl FnOnce(Pin<&mut Self>) + Send + 'static) {
        let _ = self.qt_thread().queue(emit);
    }

    /// Set the one timer to the next pause (or cancel it when none is pending). The thread is kept first, so the
    /// timer's timeout can reach this object.
    fn rearm(self: Pin<&mut Self>) {
        deadline::remember(self.qt_thread());
        match controller::with(|c| c.next_deadline()) {
            Some(at) => deadline::arm(at.saturating_sub(deadline::now_ms())),
            None => deadline::disarm(),
        }
    }
}
