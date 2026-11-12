//! The cxx-qt bridge of [`crate::command_line`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`). The `CommandLine` object's properties are the test hooks as
//! TestHooks.qml reads them, each typed as its option is (the "not given" values are documented on the reader).

use crate::command_line::state::CommandLineRust;

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
        /// The item to select (`--select`): -1 when none.
        #[qproperty(i32, select, cxx_name = "select", READ = read_select, FINAL)]
        /// The item to open in the viewer (`--view`): -1 when none.
        #[qproperty(i32, view, cxx_name = "view", READ = read_view, FINAL)]
        /// The viewer's zoom (`--zoom`): 0 when none.
        #[qproperty(f64, zoom, cxx_name = "zoom", READ = read_zoom, FINAL)]
        /// The mascot to show (`--mascot`): 0 for a random one.
        #[qproperty(i32, mascot, cxx_name = "mascot", READ = read_mascot, FINAL)]
        /// The search result to show at full size (`--yandex-preview`): -1 when none.
        #[qproperty(i32, yandex_preview, cxx_name = "yandexPreview", READ = read_yandex_preview, FINAL)]
        /// The ledger whose inbox entry to open (`--inbox-expand`): empty when none.
        #[qproperty(QString, inbox_expand, cxx_name = "inboxExpand", READ = read_inbox_expand, FINAL)]
        /// The viewer's keys (`--viewer-keys`), each a key or a rating (`r3`).
        #[qproperty(QStringList, viewer_keys, cxx_name = "viewerKeys", READ = read_viewer_keys, FINAL)]
        /// The grid's keys (`--grid-keys`): L, R, U, D.
        #[qproperty(QStringList, grid_keys, cxx_name = "gridKeys", READ = read_grid_keys, FINAL)]
        /// Open the views menu (`--menu=views`).
        #[qproperty(bool, menu_views, cxx_name = "menuViews", READ = read_menu_views, FINAL)]
        /// Open the library folders (`--library=1`).
        #[qproperty(bool, library, cxx_name = "library", READ = read_library, FINAL)]
        /// Open the save-view dialog (`--save-view=1`).
        #[qproperty(bool, save_view, cxx_name = "saveView", READ = read_save_view, FINAL)]
        /// Open the log view (`--log=1`, in dev mode).
        #[qproperty(bool, log, cxx_name = "log", READ = read_log, FINAL)]
        /// Open the inbox (`--inbox=1`).
        #[qproperty(bool, inbox, cxx_name = "inbox", READ = read_inbox, FINAL)]
        /// A Yandex search is asked for (`--find-similar`, with a path).
        #[qproperty(bool, find_similar, cxx_name = "findSimilar", READ = read_find_similar, FINAL)]
        /// The window is being photographed (`--screenshot`): its carets are not drawn, since they blink.
        #[qproperty(bool, screenshot, cxx_name = "screenshot", READ = read_screenshot, FINAL)]
        /// The folder picker opens (`--add-folder`, with a path).
        #[qproperty(bool, add_folder, cxx_name = "addFolder", READ = read_add_folder, FINAL)]
        /// The save picker opens (`--save-crops`, with a path).
        #[qproperty(bool, save_crops, cxx_name = "saveCrops", READ = read_save_crops, FINAL)]
        type CommandLine = super::CommandLineRust;
    }

    unsafe extern "RustQt" {
        fn read_select(self: &CommandLine) -> i32;
        fn read_view(self: &CommandLine) -> i32;
        fn read_zoom(self: &CommandLine) -> f64;
        fn read_mascot(self: &CommandLine) -> i32;
        fn read_yandex_preview(self: &CommandLine) -> i32;
        fn read_inbox_expand(self: &CommandLine) -> QString;
        fn read_viewer_keys(self: &CommandLine) -> QStringList;
        fn read_grid_keys(self: &CommandLine) -> QStringList;
        fn read_menu_views(self: &CommandLine) -> bool;
        fn read_library(self: &CommandLine) -> bool;
        fn read_save_view(self: &CommandLine) -> bool;
        fn read_log(self: &CommandLine) -> bool;
        fn read_inbox(self: &CommandLine) -> bool;
        fn read_find_similar(self: &CommandLine) -> bool;
        fn read_screenshot(self: &CommandLine) -> bool;
        fn read_add_folder(self: &CommandLine) -> bool;
        fn read_save_crops(self: &CommandLine) -> bool;
    }
}
