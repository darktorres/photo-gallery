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

pub(crate) mod state;
use state::ShellRust;

use crate::shell_bridge::qobject;

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use pg_client::controller::listing::Event as ListingEvent;
use pg_client::controller::panels;
use pg_client::controller::size;
use pg_client::controller::{Command, Event};
use pg_client::sort::choice_index;
use pg_model::ids::FileId;
use pg_model::notice::{Notice, ViewOp};
use pg_model::rating::Rating;
use pg_model::rules::Path as RulePath;
use pg_model::size::{Factor, GridWidth, RowHeight, Scale};
use pg_model::view::SavedView;

use crate::{app, controller, deadline, folders, gallery, keymap, library};

impl Default for ShellRust {
    /// The controller's start, mirrored: the listing the window starts with, and the size's defaults (the window sets
    /// the size and the scale when it is made).
    fn default() -> ShellRust {
        let listing = controller::listing();
        let size = controller::with(|c| *c.grid_size());
        let panels = controller::with(|c| c.panels().clone());
        ShellRust {
            folder: QString::from(&pg_platform::native::PATH_RULES.display_merged(&listing.folder)),
            recursive: listing.recursive,
            folder_kind: folders::listing_kind(&listing.folder, listing.recursive),
            descending: listing.sort.descending,
            saved_views: view_names(),
            filters_open: panels.filters_open(),
            show_borders: panels.show_borders(),
            row_height: px(size.row_height().get()),
            min_row_height: px(RowHeight::MIN.get()),
            max_row_height: px(RowHeight::MAX.get()),
            dev_mode: panels.dev_mode(),
        }
    }
}

/// A list index as QML takes it (-1 for none).
fn to_i32(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(-1)
}

/// A pixel size as QML takes it. The sizes are at most 2048, so the conversion never saturates.
fn px(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// A size value the window sent that is not a valid one: logged and dropped, never applied.
fn refused(what: &str, value: f64) {
    pg_log::warn("size", &format!("{what} {value} is not a valid value; ignored"));
}

/// The listing change a click on a tree row makes. Recursion is the listing's one toggle, so the first two rows set it:
/// "All" is the empty folder with its subfolders, "Drive roots" the empty folder without them (the drive roots' own
/// files). A folder keeps the recursion as it is.
fn row_event(kind: folders::RowKind) -> ListingEvent {
    match kind {
        folders::RowKind::All => ListingEvent::OpenAll,
        folders::RowKind::DriveRoots => ListingEvent::OpenRoots,
        folders::RowKind::Folder(folder) => ListingEvent::OpenFolder(folder),
    }
}

/// The preferences a command saves, carried out: the one place a save is made, so a test runs the same code the window
/// does. A save that fails is the preferences' to report, and the value holds for this run either way.
fn prefs_effect(command: &Command, prefs: &pg_app::PrefStore) {
    match command {
        // Dev mode the user turned on or off (a start's value is not saved).
        Command::SaveDevMode(on) => {
            let _ = prefs.update(|p| p.dev_mode = *on);
        }
        // The borders the user toggled (a start's `--borders` is not saved).
        Command::SaveShowBorders(on) => {
            let _ = prefs.update(|p| p.show_borders = *on);
        }
        // The file the window shows now (none: none is remembered), once the start is over. Read here, on the window's
        // thread, so a save at shutdown (`flush`) sees the same file the window does.
        Command::SaveLastFile => {
            let file = gallery::shown_file();
            let _ = prefs.update(|p| p.last_file = file);
        }
        // The row height, settled after a pause.
        Command::Size(size::Change::SaveRowHeight(row)) => {
            let _ = prefs.update(|p| p.row_height = *row);
        }
        _ => {}
    }
}

/// The inbox says that the change `op` to the saved views failed, for `error`.
fn views_failed(op: ViewOp, error: &impl std::fmt::Display) {
    app::post(Notice::ViewsFailed { op, error: app::error_text(error) });
}

/// The names of the saved views, sorted (`saved_views`).
fn view_names() -> QStringList {
    let views = match app::get().views().all() {
        Ok(views) => {
            app::resolved_part("views-failed", ViewOp::List.code());
            views
        }
        Err(e) => {
            views_failed(ViewOp::List, &e);
            Vec::new()
        }
    };
    views.iter().map(|v| QString::from(&v.name)).collect()
}

impl qobject::Shell {
    /// The folder tree is in. The listed folder is kept as the tree spells it, when the tree shows it under another
    /// spelling (the controller remembers that), and then the start is told whether the folder is known.
    pub(crate) fn tree_loaded(mut self: Pin<&mut Self>, folder_known: bool) {
        let listing = controller::listing();
        if let Some(spelled) = folders::spelling_of(&listing.folder, listing.recursive) {
            self.as_mut().dispatch(Event::FolderSpelled(spelled));
        }
        self.dispatch(Event::TreeLoaded { folder_known });
    }

    /// The list shown is the grid model's last applied (`gallery::APPLIED`, on this thread): its generation tells the
    /// controller which `OpenList` it is, so a reveal waits for its own.
    pub(crate) fn list_shown(self: Pin<&mut Self>, count: i32) {
        let count = usize::try_from(count).unwrap_or(0);
        self.dispatch(Event::ListShown { count, generation: gallery::applied() });
    }

    pub(crate) fn list_positioned(self: Pin<&mut Self>) {
        self.dispatch(Event::ListPositioned);
    }

    pub(crate) fn current_changed(self: Pin<&mut Self>, index: i32) {
        self.dispatch(Event::CurrentChanged(usize::try_from(index).ok()));
    }

    pub(crate) fn key_pressed(self: Pin<&mut Self>, key: i32, modifiers: i32) -> bool {
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

    pub(crate) fn viewer_step(&self, from: i32, d: i32) -> i32 {
        let from = usize::try_from(from).unwrap_or(0);
        to_i32(controller::with(|c| c.viewer_step(from, d)))
    }

    /// The grid's width. A width of zero or less is a list with no width yet (the grid's margins can make one): no width,
    /// and not logged. A value that is not a number is refused.
    pub(crate) fn grid_resized(self: Pin<&mut Self>, width: f64) {
        if !width.is_finite() {
            refused("grid width", width);
            return;
        }
        self.dispatch(Event::GridResized(GridWidth::new(width)));
    }

    pub(crate) fn query_edited(self: Pin<&mut Self>) {
        self.dispatch(Event::QueryEdited);
    }

    pub(crate) fn size_chosen(self: Pin<&mut Self>, height: f64) {
        match RowHeight::from_finite(height) {
            Some(row) => self.dispatch(Event::Size(size::Event::Chosen(row))),
            None => refused("row height", height),
        }
    }

    pub(crate) fn size_stepped(self: Pin<&mut Self>, factor: f64) {
        match Factor::new(factor) {
            Some(factor) => self.dispatch(Event::Size(size::Event::Factor(factor))),
            None => refused("size factor", factor),
        }
    }

    pub(crate) fn set_scale(self: Pin<&mut Self>, scale: f64) {
        match Scale::new(scale) {
            Some(scale) => self.dispatch(Event::Size(size::Event::Scale(scale))),
            None => refused("scale", scale),
        }
    }

    pub(crate) fn toggle_filters(self: Pin<&mut Self>) {
        self.dispatch(Event::Panel(panels::Event::ToggleFilters));
    }

    pub(crate) fn toggle_borders(self: Pin<&mut Self>) {
        self.dispatch(Event::Panel(panels::Event::ToggleBorders));
    }

    pub(crate) fn set_viewer_open(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Panel(panels::Event::SetViewerOpen(on)));
    }

    pub(crate) fn global_action(self: Pin<&mut Self>, code: i32) {
        if let Some(global) = keymap::global_of(code) {
            self.dispatch(Event::Global(global));
        }
    }

    pub(crate) fn panel_opened(self: Pin<&mut Self>, kind: i32) {
        if let Some(kind) = keymap::panel_kind_of(kind) {
            self.dispatch(Event::Panel(panels::Event::PanelOpened(kind)));
        }
    }

    pub(crate) fn panel_closed(self: Pin<&mut Self>, kind: i32) {
        if let Some(kind) = keymap::panel_kind_of(kind) {
            self.dispatch(Event::Panel(panels::Event::PanelClosed(kind)));
        }
    }

    /// Whether the window has work still to show: the core's pauses, the inbox's writes (its marks and the listing
    /// after them), and the expanded ledger's list being read.
    pub(crate) fn pending(&self) -> bool {
        controller::with(|c| c.pending()) || !app::get().inbox().idle() || crate::failed_files::loading()
    }

    /// Shutdown: the saves that wait run now; a pause that would only show the list does not.
    pub(crate) fn flush(self: Pin<&mut Self>) {
        self.dispatch(Event::Flush);
    }

    /// `rating` is 0 to 5 stars (0 takes the rating away); out of that range, nothing is rated.
    pub(crate) fn rate(self: Pin<&mut Self>, item: i32, rating: i32) {
        let rating = u8::try_from(rating).ok().and_then(Rating::new);
        if let (Some(item), Some(rating)) = (crate::rows::row_of(item), rating) {
            self.dispatch(Event::Rate { item, rating });
        }
    }

    pub(crate) fn turn(self: Pin<&mut Self>, item: i32, turns: i32) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::Turn { item, turns });
        }
    }

    pub(crate) fn toggle_border_mark(self: Pin<&mut Self>, item: i32) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::ToggleBorderMark(item));
        }
    }

    pub(crate) fn set_border_mark(self: Pin<&mut Self>, item: i32, marked: bool) {
        if let Ok(item) = usize::try_from(item) {
            self.dispatch(Event::SetBorderMark { item, marked });
        }
    }

    pub(crate) fn files_changed(self: Pin<&mut Self>) {
        self.dispatch(Event::FilesChanged);
    }

    pub(crate) fn reload_list(self: Pin<&mut Self>) {
        self.dispatch(Event::Reload);
    }

    /// The timer fired (the invokable above, queued from `deadline.cpp`): the pauses that are due run.
    pub(crate) fn timeout(self: Pin<&mut Self>) {
        self.dispatch(Event::Timeout);
    }

    pub(crate) fn open_folder_row(self: Pin<&mut Self>, row: i32) {
        self.dispatch(Event::Listing(row_event(folders::row_kind(row))));
    }

    pub(crate) fn set_recursive(self: Pin<&mut Self>, on: bool) {
        self.dispatch(Event::Listing(ListingEvent::Recursive(on)));
    }

    pub(crate) fn toggle_descending(self: Pin<&mut Self>) {
        self.dispatch(Event::Listing(ListingEvent::DescendingToggled));
    }

    pub(crate) fn sort_by(self: Pin<&mut Self>, index: i32) {
        if let Some(index) = crate::rows::row_of(index) {
            self.dispatch(Event::Listing(ListingEvent::SortChoice(index)));
        }
    }

    pub(crate) fn sort_index(&self) -> i32 {
        crate::rows::to_row(Some(choice_index(controller::listing().sort.key)))
    }

    pub(crate) fn save_view(mut self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let view = SavedView { name, listing: controller::listing() };
        match app::get().views().save(&view) {
            Ok(()) => app::resolved_part("views-failed", ViewOp::Save.code()),
            Err(e) => views_failed(ViewOp::Save, &e),
        }
        self.as_mut().set_saved_views(view_names());
    }

    pub(crate) fn delete_view(mut self: Pin<&mut Self>, name: &QString) {
        match app::get().views().delete(&name.to_string()) {
            Ok(()) => app::resolved_part("views-failed", ViewOp::Delete.code()),
            Err(e) => views_failed(ViewOp::Delete, &e),
        }
        self.as_mut().set_saved_views(view_names());
    }

    /// Show the file `file_id` in its folder, nothing filtered (an inbox entry's Reveal). Its folder is read off the
    /// window's thread (`pg_app::Session::reveal_folder`); when it is delivered, the controller lists the folder and
    /// selects the file once its list is shown (`Command::Reveal`), and `revealShown` is emitted. A file the library
    /// no longer has is a toast instead; a newer Reveal supersedes this one, whose delivery is dropped.
    pub(crate) fn reveal(self: Pin<&mut Self>, file_id: f64) {
        // The bridge's number becomes a file's id here, once: everything after it is typed.
        let file = FileId::new(pg_model::num::f64_to_i64(file_id));
        let shell = self.qt_thread();
        app::get().reading().session().reveal_folder(file, move |folder| {
            // Fails only when the window is gone: nothing left to reveal in.
            let _ = shell.queue(move |shell| shell.revealed(file, folder));
        });
    }

    /// A Reveal's folder, delivered on the Qt thread: listed, or a toast when the file is no longer in the library.
    fn revealed(mut self: Pin<&mut Self>, file: FileId, folder: pg_app::Result<Option<RulePath>>) {
        match folder {
            Ok(Some(folder)) => {
                let folder = pg_platform::native::PATH_RULES.merge(&folder);
                self.as_mut().dispatch(Event::Reveal { file, folder });
                self.later(|shell| shell.reveal_shown());
            }
            Ok(None) => app::post(Notice::FileNoLongerInLibrary),
            Err(e) => pg_log::warn("inbox", &format!("a file to reveal couldn't be read: {e}")),
        }
    }

    /// Show the folder above the scans' failed folder at `row` of the expanded entry's list (`failed_files`), nothing
    /// filtered: the folder itself couldn't be read, so its parent is where it shows. Then `revealShown`.
    pub(crate) fn reveal_folder(mut self: Pin<&mut Self>, row: i32) {
        let Some(folder) = crate::rows::row_of(row).and_then(crate::failed_files::shown_folder) else { return };
        let rules = pg_platform::native::PATH_RULES;
        let shown = rules.split_file_name(&folder).map_or(folder, |(parent, _)| parent);
        self.as_mut().dispatch(Event::Listing(ListingEvent::Reveal(rules.merge(&shown))));
        self.later(|shell| shell.reveal_shown());
    }

    /// List the files the import failed on, in every folder (an inbox entry's "Show the failed files").
    pub(crate) fn show_failed_imports(self: Pin<&mut Self>) {
        self.dispatch(Event::Listing(ListingEvent::FailedImports));
    }

    pub(crate) fn apply_view(self: Pin<&mut Self>, name: &QString) -> bool {
        let Some(v) = app::get().views().view(&name.to_string()).ok().flatten() else {
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
        let folder = QString::from(&pg_platform::native::PATH_RULES.display_merged(&listing.folder));
        if self.folder != folder {
            self.as_mut().rust_mut().folder = folder;
            self.as_mut().folder_changed();
        }
        if self.recursive != listing.recursive {
            self.as_mut().rust_mut().recursive = listing.recursive;
            self.as_mut().recursive_changed();
        }
        // The label follows the tree too (a tree read after a scan can add the drive roots' files): `dispatch` mirrors
        // on every event, the tree's arrival included.
        let folder_kind = folders::listing_kind(&listing.folder, listing.recursive);
        if self.folder_kind != folder_kind {
            self.as_mut().rust_mut().folder_kind = folder_kind;
            self.as_mut().folder_kind_changed();
        }
        if self.descending != listing.sort.descending {
            self.as_mut().rust_mut().descending = listing.sort.descending;
            self.as_mut().descending_changed();
        }
    }

    /// Carry out one of the controller's commands.
    fn execute(mut self: Pin<&mut Self>, command: Command) {
        prefs_effect(&command, app::get().prefs());
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
            // The file a reveal asked for, in the list just shown: selected and shown, or gone since.
            Command::Reveal(file) => {
                let shell = self.qt_thread();
                gallery::post(move |mut model| {
                    let item = model.index_of(file.get());
                    if model.is_item(item) {
                        model.as_mut().set_current_item(item);
                        let _ = shell.queue(move |s| s.select_requested(item));
                    } else {
                        app::post(Notice::FileNoLongerInLibrary);
                    }
                });
            }
            // The session's last file, selected in the list just shown if the list has it.
            Command::RestoreSelection(file) => {
                gallery::post(move |mut model| {
                    let item = model.index_of(file.get());
                    if model.is_item(item) {
                        model.as_mut().set_current_item(item);
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
            Command::OpenList { listing, keep, generation } => {
                gallery::post(move |model| model.open_listing(listing, keep, generation));
                self.as_mut().later(|shell| shell.listing_changed());
            }
            Command::Rate { item, rating } => {
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
            Command::SaveSession(listing) => match app::get().views().save_listing(&listing) {
                Ok(()) => app::resolved("session-not-saved"),
                Err(e) => app::post(Notice::SessionNotSaved { error: app::error_text(&format!("{e:#}")) }),
            },
            // Saved by `prefs_effect`, above, from the file the window shows now.
            Command::SaveLastFile => {}
            Command::Relayout { .. } => self.as_mut().later(|shell| shell.relayout_due()),
            Command::Panel(change) => self.as_mut().panel(change),
            Command::OpenPanel(kind) => {
                let kind = keymap::panel_kind_code(kind);
                self.as_mut().later(move |shell| shell.panel_open_requested(kind));
            }
            Command::ClosePanel(kind) => {
                let kind = keymap::panel_kind_code(kind);
                self.as_mut().later(move |shell| shell.panel_close_requested(kind));
            }
            // Saved by `prefs_effect`, above.
            Command::SaveDevMode(_) | Command::SaveShowBorders(_) => {}
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
            panels::Change::DevMode(on) => {
                self.as_mut().rust_mut().dev_mode = on;
                self.as_mut().dev_mode_changed();
            }
        }
    }

    /// A size's change: the row height is mirrored for the grid's layout, the thumbnail height goes to the grid model and the
    /// library (they make the thumbnail URLs and the thumbnails for it), and the saved row height goes to the preferences.
    fn size(mut self: Pin<&mut Self>, change: size::Change) {
        match change {
            size::Change::RowHeight(height) => {
                self.as_mut().rust_mut().row_height = px(height.get());
                self.as_mut().row_height_changed();
            }
            size::Change::ThumbHeight(height) => {
                gallery::post(move |model| model.set_thumb_height(height));
                library::set_thumb_height(height);
            }
            // Saved by `prefs_effect`, above.
            size::Change::SaveRowHeight(_) => {}
        }
    }

    /// Emit a signal after this call. Its handler in Main.qml calls back into the shell (the list, the layout, the
    /// selection), which it cannot do inside the call that emitted the signal; so the emission waits for the event loop.
    fn later(self: Pin<&mut Self>, emit: impl FnOnce(Pin<&mut Self>) + Send + 'static) {
        let _ = self.qt_thread().queue(emit);
    }

    /// Set the shell's timer to the next pause (or cancel it when none is pending). The timer is a child of this object,
    /// so its timeout queues `timeout` here.
    fn rearm(self: Pin<&mut Self>) {
        let owner = deadline::owner(self);
        match controller::with(|c| c.next_deadline()) {
            Some(at) => deadline::arm(owner, at.saturating_sub(deadline::now_ms())),
            None => deadline::disarm(owner),
        }
    }
}

#[cfg(test)]
mod tests {
    use pg_client::controller::listing;
    use pg_client::tree::Tree;
    use pg_model::query::{Query, Sort};
    use pg_model::rules::{Merged, PathRules};
    use pg_model::tree::TreeBuilder;
    use pg_model::view::Listing;

    use super::*;
    use crate::folders::RowKind;
    use pg_client::controller::{Controller, Start};
    use pg_client::keys::Global;

    /// A started window shows its panels as the start has them, and saves nothing: the preferences file is, byte for byte,
    /// what it was. The user's own toggles afterwards are saved, so the check can fail.
    #[test]
    fn a_started_window_leaves_the_prefs_file_byte_identical() {
        let home = pg_testkit::test_folder("scratch_desktop", "seeded-start").unwrap();
        let config = pg_app::Config::in_home(&home);
        let prefs_path = config.layout.prefs().to_path_buf();
        let app = pg_app::App::open(config).unwrap();
        app.prefs()
            .update(|p| {
                p.dev_mode = false;
                p.show_borders = true;
            })
            .unwrap();
        let before = std::fs::read(&prefs_path).unwrap();
        // The commands as the shell carries them out: `prefs_effect` is what writes the preferences.
        let run = |c: &mut Controller, event: Event| -> Vec<Command> {
            let commands = c.update(0, event);
            for command in &commands {
                prefs_effect(command, app.prefs());
            }
            commands
        };
        let row = RowHeight::new(200).expect("a row height in range");
        let start = Start {
            show_borders: true,
            dev_mode: true,
            filters_open: true,
            ..Start::new(Listing::default(), row, row)
        };
        let mut c = Controller::new(start);
        assert!(c.panels().show_borders() && c.panels().dev_mode() && c.panels().filters_open());
        run(&mut c, Event::Size(size::Event::Scale(pg_model::size::Scale::new(1.0).expect("a scale"))));
        assert_eq!(std::fs::read(&prefs_path).unwrap(), before, "a start saves nothing");
        run(&mut c, Event::Panel(panels::Event::ToggleBorders));
        assert!(!app.prefs().get().show_borders, "the user's border toggle is saved");
        assert_ne!(std::fs::read(&prefs_path).unwrap(), before);
        let dev = run(&mut c, Event::Global(Global::DevMode));
        assert!(dev.contains(&Command::SaveDevMode(false)), "the user's dev switch is saved: {dev:?}");
        app.shutdown();
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A start with every start option set saves nothing: the preferences file and the session are as they were when the
    /// start is over. The user's own change after it is saved, so the check can fail.
    #[test]
    fn a_start_with_its_options_saves_nothing() {
        let home = pg_testkit::test_folder("scratch_desktop", "start-options").unwrap();
        let config = pg_app::Config::in_home(&home);
        let prefs_path = config.layout.prefs().to_path_buf();
        let app = pg_app::App::open(config).unwrap();
        app.prefs()
            .update(|p| {
                p.show_borders = false;
                p.dev_mode = false;
            })
            .unwrap();
        let session = Listing { query: Query { name: "session".into(), ..Query::default() }, ..Listing::default() };
        app.views().save_listing(&session).unwrap();
        let prefs_before = std::fs::read(&prefs_path).unwrap();
        let session_before = app.views().listing().unwrap();
        let options = crate::command_line::StartOptions {
            query: Some(Query { name: "option".into(), ..Query::default() }),
            sort: Some(Sort { key: pg_model::query::SortKey::Size, descending: true }),
            apply_view: None,
            size: Some(RowHeight::new(120).expect("a row height in range")),
            filters_open: Some(true),
            show_borders: Some(true),
            dev_mode: Some(true),
        };
        let start = crate::controller::start_state(&app.prefs().get(), &options, app.views().listing().unwrap(), None);
        let mut c = Controller::new(start);
        // The commands as the shell carries them out: the preferences by `prefs_effect`, the session as `execute` saves it.
        let mut carried = Vec::new();
        let carry = |command: &Command| {
            prefs_effect(command, app.prefs());
            if let Command::SaveSession(listing) = command {
                app.views().save_listing(listing).unwrap();
            }
        };
        let start_events = [
            (0, Event::Size(size::Event::Scale(pg_model::size::Scale::new(1.0).expect("a scale")))),
            (1, Event::TreeLoaded { folder_known: true }),
            (2, Event::ListShown { count: 10, generation: None }),
            (3, Event::ListPositioned),
        ];
        for (now, event) in start_events {
            for command in c.update(now, event) {
                carry(&command);
                carried.push(command);
            }
        }
        assert!(carried.contains(&Command::Started), "{carried:?}");
        assert_eq!(std::fs::read(&prefs_path).unwrap(), prefs_before, "the start saves no preference");
        assert_eq!(app.views().listing().unwrap(), session_before, "the start saves no session");
        for command in c.update(4, Event::Panel(panels::Event::ToggleBorders)) {
            carry(&command);
        }
        assert!(!app.prefs().get().show_borders, "the user's border toggle is saved");
        app.shutdown();
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A row height the user settles on is saved by `prefs_effect` (the controller's `SaveRowHeight`, after its pause), and
    /// one that is only shown (a start's `--size`) is not: the controller never names it to save.
    #[test]
    fn a_settled_row_height_is_saved_and_a_start_size_is_not() {
        let home = pg_testkit::test_folder("scratch_desktop", "settled-row").unwrap();
        let config = pg_app::Config::in_home(&home);
        let app = pg_app::App::open(config).unwrap();
        let saved = RowHeight::new(220).expect("a row height in range");
        let size = RowHeight::new(120).expect("a row height in range");
        let mut c = Controller::new(Start::new(Listing::default(), size, saved));
        c.update(0, Event::Size(size::Event::Scale(pg_model::size::Scale::new(1.0).expect("a scale"))));
        let mut saves = Vec::new();
        for command in c.update(1, Event::Timeout).into_iter().chain(c.update(2, Event::Flush)) {
            prefs_effect(&command, app.prefs());
            saves.push(command);
        }
        assert_eq!(app.prefs().get().row_height, saved, "the start size is not saved: {saves:?}");
        let user = c.update(3, Event::Size(size::Event::Chosen(RowHeight::new(300).expect("a row height in range"))));
        assert!(
            user.contains(&Command::Size(size::Change::RowHeight(RowHeight::new(300).expect("a row height in range"))))
        );
        for command in c.update(400, Event::Timeout) {
            prefs_effect(&command, app.prefs());
        }
        assert_eq!(app.prefs().get().row_height, RowHeight::new(300).expect("a row height in range"));
        app.shutdown();
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A tree with files directly in `D:\` and in the folder `D:\p`: rows "All", "Drive roots", `p`.
    fn tree() -> Tree {
        let rules = PathRules::Windows;
        let mut builder = TreeBuilder::with_capacity(rules, 2, 0, 0);
        builder.add(1, &rules.parse(r"D:\").unwrap(), 2);
        builder.add(2, &rules.parse(r"D:\p").unwrap(), 3);
        let mut tree = Tree::new(rules);
        tree.replace(builder.build());
        tree
    }

    /// The listing a click on `row` leaves, from a recursive listing of the folder `q`.
    fn clicked(tree: &Tree, row: i32) -> (ListingEvent, Listing) {
        let event = row_event(RowKind::of(tree, row));
        let folder = Merged::from(vec![pg_model::name::Name::from("q")]);
        let mut shown = Listing { folder, recursive: true, sort: Sort::default(), query: Query::default() };
        listing::update(&mut shown, event.clone());
        (event, shown)
    }

    /// A click on "Drive roots" opens the drive roots' own files (`OpenRoots`: the folder `""` without subfolders),
    /// not the empty folder with recursion left on, which would list every file.
    #[test]
    fn the_drive_roots_row_opens_the_roots_without_subfolders() {
        let tree = tree();
        assert_eq!(RowKind::of(&tree, 1), RowKind::DriveRoots);
        let (event, shown) = clicked(&tree, 1);
        assert_eq!(event, ListingEvent::OpenRoots);
        assert_ne!(event, ListingEvent::OpenFolder(Merged::default()));
        assert!(shown.folder.is_empty() && !shown.recursive);
    }

    /// The label kind of the listed folder (L-26): the empty folder is "Drive roots" only without subfolders and with
    /// root files (the tree's rule), so a listing of `("", false)` with no root files is "All folders", as the tree's
    /// highlighted row is; a folder is a folder.
    #[test]
    fn the_label_of_the_empty_folder_follows_the_tree_rule() {
        let all = Merged::default();
        let p = Merged::from(vec![pg_model::name::Name::from("p")]);
        let with_roots = tree();
        assert_eq!(folders::listing_kind_of(&with_roots, &all, true), folders::KIND_ALL);
        assert_eq!(folders::listing_kind_of(&with_roots, &all, false), folders::KIND_DRIVE_ROOTS);
        assert_eq!(folders::listing_kind_of(&with_roots, &p, false), folders::KIND_FOLDER);
        assert_eq!(folders::listing_kind_of(&with_roots, &p, true), folders::KIND_FOLDER);

        let rules = PathRules::Windows;
        let mut builder = TreeBuilder::with_capacity(rules, 1, 0, 0);
        builder.add(2, &rules.parse(r"D:\p").unwrap(), 3);
        let mut without_roots = Tree::new(rules);
        without_roots.replace(builder.build());
        assert_eq!(folders::listing_kind_of(&without_roots, &all, false), folders::KIND_ALL);
        assert_eq!(without_roots.row_of(&all, false), Some(0));
        // A folder the tree lacks is still a folder: its name is the label.
        let nope = Merged::from(vec![pg_model::name::Name::from("nope")]);
        assert_eq!(folders::listing_kind_of(&without_roots, &nope, false), folders::KIND_FOLDER);
    }

    /// "All" (and a row the tree doesn't have) opens every folder with its subfolders; a folder's row opens that
    /// folder, recursion as it was.
    #[test]
    fn the_all_row_opens_every_folder_and_a_folder_row_its_folder() {
        let tree = tree();
        for row in [0, -1, 99] {
            let (event, shown) = clicked(&tree, row);
            assert_eq!(event, ListingEvent::OpenAll, "row {row}");
            assert!(shown.folder.is_empty() && shown.recursive, "row {row}");
        }
        let p = Merged::from(vec![pg_model::name::Name::from("p")]);
        let (event, shown) = clicked(&tree, 2);
        assert_eq!(event, ListingEvent::OpenFolder(p.clone()));
        assert_eq!((shown.folder, shown.recursive), (p, true));
    }
}
