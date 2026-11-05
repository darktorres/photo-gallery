//! The window's shell: the one object the window reports its events to, and the core's policy behind it
//! (`pg_client::controller`). It holds the sub-machines and the pauses (deadlines), carries their commands out as signals
//! and qproperties that Main.qml routes to the presentation, and drives the one timer (`deadline.rs`). It also holds the
//! listing's view of the window (the folder, subfolders, sort and saved views: `listing.rs`), changed through the
//! listing's policy and announced as `listingChanged` when a list is to be shown.
//!
//! Re-entrance: a signal is emitted within the call that caused it, and so is a qproperty's notify. A QML handler of one
//! of these signals or notifies must not call the shell back synchronously; it defers the call with `Qt.callLater`.

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
        /// The merged folder shown (`""` = all folders). Setting it only spells it (the tree's spelling); to show a
        /// folder, `openFolder`.
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

        /// The listed folder is gone: list all folders.
        #[qsignal]
        #[cxx_name = "openAll"]
        fn open_all(self: Pin<&mut Shell>);

        /// Select this item of the list (the session's last file, at the start).
        #[qsignal]
        #[cxx_name = "selectItem"]
        fn select_item(self: Pin<&mut Shell>, index: i32);

        /// The start is over: the test hooks run and the background job starts.
        #[qsignal]
        fn started(self: Pin<&mut Shell>);

        /// The current item changed (-1: none).
        #[qinvokable]
        #[cxx_name = "currentChanged"]
        fn current_changed(self: Pin<&mut Shell>, index: i32);

        /// A grid key was pressed (Qt's key code and modifiers). True when the grid uses it: its action is then sent as
        /// `selectRequested`, `stepRequested`, `gridAction` or `viewerRequested`, or it changes the size or a panel.
        #[qinvokable]
        #[cxx_name = "keyPressed"]
        fn key_pressed(self: Pin<&mut Shell>, key: i32, modifiers: i32) -> bool;

        /// The item a viewer step from `from` by `d` lands on, clamped to the list (the viewer moves there).
        #[qinvokable]
        #[cxx_name = "viewerStep"]
        fn viewer_step(self: Pin<&mut Shell>, from: i32, d: i32) -> i32;

        /// The grid's width changed: a relayout is due after a short pause (`relayoutDue`).
        #[qinvokable]
        #[cxx_name = "gridResized"]
        fn grid_resized(self: Pin<&mut Shell>);

        /// The filter bar's query was edited: the list is shown after a pause (`listDue`).
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

        /// Whether a pause is still running (the screenshot waits for none).
        #[qinvokable]
        #[cxx_name = "pending"]
        fn pending(self: Pin<&mut Shell>) -> bool;

        /// Shutdown: the pending saves run now (the session and the row height are kept).
        #[qinvokable]
        #[cxx_name = "flush"]
        fn flush(self: Pin<&mut Shell>);

        /// A grid action for the current item: a `GridAction` code (Main.qml) and its argument (the rating).
        #[qsignal]
        #[cxx_name = "gridAction"]
        fn grid_action(self: Pin<&mut Shell>, action: i32, arg: i32);

        /// Select this item and scroll it into view.
        #[qsignal]
        #[cxx_name = "selectRequested"]
        fn select_requested(self: Pin<&mut Shell>, index: i32);

        /// Select the neighbour of the current item `dx` columns and `dy` rows away (the grid's layout resolves it).
        #[qsignal]
        #[cxx_name = "stepRequested"]
        fn step_requested(self: Pin<&mut Shell>, dx: i32, dy: i32);

        /// Show the viewer at this item.
        #[qsignal]
        #[cxx_name = "viewerRequested"]
        fn viewer_requested(self: Pin<&mut Shell>, index: i32);

        /// The list is shown after its pause (the query was edited).
        #[qsignal]
        #[cxx_name = "listDue"]
        fn list_due(self: Pin<&mut Shell>);

        /// The selection rested: remember the file it is on.
        #[qsignal]
        #[cxx_name = "sessionDue"]
        fn session_due(self: Pin<&mut Shell>);

        /// The grid's width settled: lay the grid out again.
        #[qsignal]
        #[cxx_name = "relayoutDue"]
        fn relayout_due(self: Pin<&mut Shell>);

        /// Remember this row height for the next start.
        #[qsignal]
        #[cxx_name = "saveRowHeight"]
        fn save_row_height(self: Pin<&mut Shell>, height: i32);

        /// The listing changed in a way that opens a list (a folder, subfolders, sort, query or saved view): show it.
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
        #[cxx_name = "saveSession"]
        fn save_session(self: &Shell);

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

        /// The viewer is open or shut: while it is open, the grid does not take the keys (`keyPressed` is false).
        #[qinvokable]
        #[cxx_name = "setViewerOpen"]
        fn set_viewer_open(self: Pin<&mut Shell>, on: bool);
    }
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_client::controller::deadlines::{Deadlines, Timer};
use pg_client::controller::input::{self, Input};
use pg_client::controller::listing::{Command as ListingCommand, Event as ListingEvent};
use pg_client::controller::panels::{self, Panels};
use pg_client::controller::prefs::{self, Prefs};
use pg_client::controller::start::{self, StartGate};
use pg_client::keys::Grid;
use pg_client::sort::choice_index;
use pg_model::view::SavedView;

use crate::{app, deadline, keymap, listing};

pub struct ShellRust {
    gate: StartGate,
    input: Input,
    panels: Panels,
    prefs: Prefs,
    deadlines: Deadlines,
    filters_open: bool,
    show_borders: bool,
    row_height: i32,
    thumb_height: i32,
    folder: QString,
    recursive: bool,
    descending: bool,
    saved_views: QStringList,
}

impl Default for ShellRust {
    /// The defaults of the core, mirrored: the desktop sets the size and the scale when the window starts. The listing
    /// is the one the window starts with (`listing::get`), shown.
    fn default() -> ShellRust {
        let prefs = Prefs::default();
        let listing = listing::get();
        ShellRust {
            gate: StartGate::default(),
            input: Input::default(),
            panels: Panels::default(),
            row_height: prefs.row_height(),
            thumb_height: prefs.thumb_height(),
            prefs,
            deadlines: Deadlines::default(),
            filters_open: false,
            show_borders: false,
            folder: QString::from(&listing.folder),
            recursive: listing.recursive,
            descending: listing.sort.descending,
            saved_views: view_names(),
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
            eprintln!("views: {e:#}");
            Vec::new()
        })
        .iter()
        .map(|v| QString::from(&v.name))
        .collect()
}

impl qobject::Shell {
    fn tree_loaded(self: Pin<&mut Self>, folder_known: bool) {
        self.run(start::Event::TreeLoaded { folder_known });
    }

    fn list_shown(mut self: Pin<&mut Self>, last_file: i32, count: i32) {
        let count = usize::try_from(count).unwrap_or(0);
        self.as_mut().input(input::Event::ListShown { count });
        let last_file = usize::try_from(last_file).ok();
        self.run(start::Event::ListShown { last_file });
    }

    fn list_positioned(self: Pin<&mut Self>) {
        self.run(start::Event::ListPositioned);
    }

    fn current_changed(mut self: Pin<&mut Self>, index: i32) {
        let current = usize::try_from(index).ok();
        self.as_mut().input(input::Event::CurrentChanged(current));
        self.arm(Timer::SaveSession);
    }

    fn key_pressed(mut self: Pin<&mut Self>, key: i32, modifiers: i32) -> bool {
        // While the viewer is open, its keys are its own: the grid leaves them.
        if self.as_mut().rust_mut().panels.viewer_open() {
            return false;
        }
        let has_current = self.as_mut().rust_mut().input.has_current();
        let Some(grid) = keymap::grid_key(key, modifiers, has_current) else {
            return false;
        };
        self.input(input::Event::Key(grid));
        true
    }

    fn viewer_step(mut self: Pin<&mut Self>, from: i32, d: i32) -> i32 {
        let from = usize::try_from(from).unwrap_or(0);
        to_i32(self.as_mut().rust_mut().input.viewer_step(from, d))
    }

    fn grid_resized(self: Pin<&mut Self>) {
        self.arm(Timer::Layout);
    }

    fn query_edited(self: Pin<&mut Self>) {
        self.arm(Timer::ApplyQuery);
    }

    fn size_chosen(self: Pin<&mut Self>, height: f64) {
        self.size(prefs::Event::Chosen(height));
    }

    fn size_stepped(self: Pin<&mut Self>, factor: f64) {
        self.size(prefs::Event::Factor(factor));
    }

    fn set_scale(self: Pin<&mut Self>, scale: f64) {
        self.size(prefs::Event::Scale(scale));
    }

    fn toggle_filters(self: Pin<&mut Self>) {
        self.panel(panels::Event::ToggleFilters);
    }

    fn set_filters_open(self: Pin<&mut Self>, on: bool) {
        self.panel(panels::Event::SetFiltersOpen(on));
    }

    fn toggle_borders(self: Pin<&mut Self>) {
        self.panel(panels::Event::ToggleBorders);
    }

    fn set_show_borders(self: Pin<&mut Self>, on: bool) {
        self.panel(panels::Event::SetShowBorders(on));
    }

    fn set_viewer_open(self: Pin<&mut Self>, on: bool) {
        self.panel(panels::Event::SetViewerOpen(on));
    }

    fn pending(mut self: Pin<&mut Self>) -> bool {
        self.as_mut().rust_mut().deadlines.pending()
    }

    /// Shutdown: the saves that are waiting run now. The list is not shown for a pause that would only show it.
    fn flush(mut self: Pin<&mut Self>) {
        let timers = self.as_mut().rust_mut().deadlines.drain();
        for timer in timers {
            if matches!(timer, Timer::SaveSession | Timer::Size) {
                self.as_mut().fire(timer);
            }
        }
        self.rearm();
    }

    /// The timer fired (queued from `deadline.rs`): run the pauses that are due, then arm the next.
    pub(crate) fn timeout(mut self: Pin<&mut Self>) {
        let now = deadline::now_ms();
        let timers = self.as_mut().rust_mut().deadlines.handle_timeout(now);
        for timer in timers {
            self.as_mut().fire(timer);
        }
        self.rearm();
    }

    /// Emit a signal after this call. Its handler in Main.qml calls back into the shell (the list, the layout, the
    /// selection), which it cannot do inside the call that emitted the signal; so the emission waits for the event loop.
    fn later(self: Pin<&mut Self>, emit: impl FnOnce(Pin<&mut Self>) + Send + 'static) {
        let _ = self.qt_thread().queue(emit);
    }

    /// The gate's commands for `event`, carried out in order as signals.
    fn run(mut self: Pin<&mut Self>, event: start::Event) {
        let commands = self.as_mut().rust_mut().gate.update(event);
        for command in commands {
            match command {
                start::Command::OpenAll => self.as_mut().later(|shell| shell.open_all()),
                start::Command::Select(i) => {
                    let i = to_i32(i);
                    self.as_mut().later(move |shell| shell.select_item(i));
                }
                start::Command::Started => self.as_mut().later(|shell| shell.started()),
            }
        }
    }

    /// The grid's policy for `event`: each command carried out, in order.
    fn input(mut self: Pin<&mut Self>, event: input::Event) {
        let commands = self.as_mut().rust_mut().input.update(event);
        for command in commands {
            self.as_mut().act(command);
        }
    }

    fn act(mut self: Pin<&mut Self>, command: input::Command) {
        match command {
            input::Command::Select(i) => {
                let i = to_i32(i);
                self.as_mut().later(move |shell| shell.select_requested(i));
            }
            input::Command::Step { dx, dy } => self.as_mut().later(move |shell| shell.step_requested(dx, dy)),
            input::Command::Act(grid) => {
                let (action, arg) = keymap::code_of(grid);
                self.as_mut().later(move |shell| shell.grid_action(action, arg));
            }
            input::Command::ShowViewer(i) => {
                let i = to_i32(i);
                self.as_mut().later(move |shell| shell.viewer_requested(i));
            }
            input::Command::Forward(grid) => self.forward(grid),
        }
    }

    /// The keys the grid hands to the size and the panels.
    fn forward(self: Pin<&mut Self>, grid: Grid) {
        match grid {
            Grid::Resize { grow } => self.size(prefs::Event::Resize { grow }),
            Grid::Filters => self.panel(panels::Event::ToggleFilters),
            Grid::Borders => self.panel(panels::Event::ToggleBorders),
            _ => {}
        }
    }

    fn size(mut self: Pin<&mut Self>, event: prefs::Event) {
        let changes = self.as_mut().rust_mut().prefs.update(event);
        self.apply_size(changes);
    }

    /// Show the size's changes: the row height (the grid re-lays out, and the thumbnails follow after a pause), the
    /// thumbnail height, and the saved row height.
    fn apply_size(mut self: Pin<&mut Self>, changes: Vec<prefs::Change>) {
        for change in changes {
            match change {
                prefs::Change::RowHeight(height) => {
                    self.as_mut().rust_mut().row_height = height;
                    self.as_mut().later(|shell| shell.row_height_changed());
                    self.as_mut().arm(Timer::Size);
                }
                prefs::Change::ThumbHeight(height) => {
                    self.as_mut().rust_mut().thumb_height = height;
                    self.as_mut().thumb_height_changed();
                }
                prefs::Change::SaveRowHeight(height) => self.as_mut().save_row_height(height),
            }
        }
    }

    fn panel(mut self: Pin<&mut Self>, event: panels::Event) {
        let changes = self.as_mut().rust_mut().panels.update(event);
        for change in changes {
            match change {
                panels::Change::FiltersOpen(on) => {
                    self.as_mut().rust_mut().filters_open = on;
                    self.as_mut().filters_open_changed();
                }
                panels::Change::ShowBorders(on) => {
                    self.as_mut().rust_mut().show_borders = on;
                    self.as_mut().show_borders_changed();
                }
                // The core holds the viewer's flag alone (`key_pressed` reads it): nothing here to show.
                panels::Change::ViewerOpen(_) => {}
            }
        }
    }

    /// Copy the listing into the properties, notifying those that changed.
    fn show_listing(mut self: Pin<&mut Self>) {
        let listing = listing::get();
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

    /// A change to the listing: applied, shown, then its commands carried out. A list is opened by `listingChanged`,
    /// queued, so the window never reads the shell while it is changing it.
    fn listing_event(mut self: Pin<&mut Self>, event: ListingEvent) {
        let commands = listing::dispatch(event);
        self.as_mut().show_listing();
        for command in commands {
            match command {
                ListingCommand::OpenList => {
                    let _ = self.qt_thread().queue(|shell| shell.listing_changed());
                }
                ListingCommand::SaveSession => listing::save_session(),
            }
        }
    }

    fn set_folder(self: Pin<&mut Self>, value: &QString) {
        listing::respell(value.to_string());
        self.show_listing();
    }

    fn open_folder(self: Pin<&mut Self>, folder: &QString) {
        self.listing_event(ListingEvent::OpenFolder(folder.to_string()));
    }

    fn set_recursive(self: Pin<&mut Self>, on: bool) {
        self.listing_event(ListingEvent::Recursive(on));
    }

    fn toggle_descending(self: Pin<&mut Self>) {
        self.listing_event(ListingEvent::DescendingToggled);
    }

    fn sort_by(self: Pin<&mut Self>, index: i32) {
        self.listing_event(ListingEvent::SortChoice(index));
    }

    fn sort_index(&self) -> i32 {
        choice_index(listing::get().sort.key)
    }

    fn save_session(&self) {
        listing::save_session();
    }

    fn save_view(self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let listing = listing::get();
        let view = SavedView {
            name,
            folder: listing.folder,
            recursive: listing.recursive,
            query: listing.query,
            sort: listing.sort,
        };
        if let Err(e) = app::get().save_view(&view) {
            eprintln!("saving view: {e:#}");
        }
        self.set_saved_views(view_names());
    }

    fn delete_view(self: Pin<&mut Self>, name: &QString) {
        if let Err(e) = app::get().delete_view(&name.to_string()) {
            eprintln!("deleting view: {e:#}");
        }
        self.set_saved_views(view_names());
    }

    fn apply_view(self: Pin<&mut Self>, name: &QString) -> bool {
        let Some(v) = app::get().view(&name.to_string()).ok().flatten() else {
            return false;
        };
        self.listing_event(ListingEvent::ViewApplied(v));
        true
    }

    /// Start `timer`'s pause now, and arm the one timer for the next deadline.
    fn arm(mut self: Pin<&mut Self>, timer: Timer) {
        let now = deadline::now_ms();
        self.as_mut().rust_mut().deadlines.arm(timer, now);
        self.rearm();
    }

    /// Carry out a pause that is due.
    fn fire(mut self: Pin<&mut Self>, timer: Timer) {
        match timer {
            Timer::ApplyQuery => self.as_mut().later(|shell| shell.list_due()),
            // Synchronous: the save must run at shutdown too (`flush`), and its handler calls no shell.
            Timer::SaveSession => self.as_mut().session_due(),
            Timer::Layout => self.as_mut().later(|shell| shell.relayout_due()),
            Timer::Size => {
                let changes = self.as_mut().rust_mut().prefs.settled();
                self.apply_size(changes);
            }
        }
    }

    /// Set the one timer to the next deadline (or cancel it when none is pending). The thread is kept first, so the
    /// timer's timeout can reach this object.
    fn rearm(mut self: Pin<&mut Self>) {
        deadline::remember(self.qt_thread());
        let now = deadline::now_ms();
        match self.as_mut().rust_mut().deadlines.next_deadline() {
            Some(at) => deadline::arm(at.saturating_sub(now)),
            None => deadline::disarm(),
        }
    }
}
