//! The cxx-qt bridge of [`crate::shell`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::shell::state::ShellRust;

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
        /// The folder shown, as its parts joined for display (the title, the top bar): `""` for all folders. Display only:
        /// the listing is the controller's, and a folder is shown by its row (`openFolderRow`).
        #[qproperty(QString, folder, READ, NOTIFY, FINAL)]
        /// With the files of its subfolders (`setRecursive`).
        #[qproperty(bool, recursive, READ, NOTIFY, FINAL)]
        /// The kind of the folder's label: 0 all folders, 1 drive roots (the empty folder without subfolders, when the
        /// drive roots hold files), 2 a folder (its name). The tree's rule (`Tree::shown_as`), for the top bar and the
        /// save dialog.
        #[qproperty(i32, folder_kind, cxx_name = "folderKind", READ, NOTIFY, FINAL)]
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
        /// The deadline timer fired (C++ queues this call on the shell's thread, `deadline.cpp`): the pauses that are due run.
        #[qinvokable]
        fn timeout(self: Pin<&mut Shell>);

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

        /// Show the folder of a tree row, with the current subfolders, sort and query. Row 0 (and a row the tree
        /// doesn't have) is all folders, with their subfolders; the "Drive roots" row is the drive roots' own files,
        /// without.
        #[qinvokable]
        #[cxx_name = "openFolderRow"]
        fn open_folder_row(self: Pin<&mut Shell>, row: i32);

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
