//! The cxx-qt bridge of [`crate::keymap`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::keymap::state::KeyMapRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type KeyMap = super::KeyMapRust;
    }

    unsafe extern "RustQt" {
        /// The viewer's action for a key press, for what it shows (`kind`: a `GalleryModel.MediaKind` value, passed as
        /// its number): a `ViewerAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "viewerAction"]
        fn viewer_action(self: &KeyMap, key: i32, modifiers: i32, kind: i32, has_borders: bool) -> i32;

        /// The argument of the viewer's action (the rating of `ViewerAction.Rate`), 0 for the others.
        #[qinvokable]
        #[cxx_name = "viewerArg"]
        fn viewer_arg(self: &KeyMap, key: i32, modifiers: i32, kind: i32, has_borders: bool) -> i32;

        #[qinvokable]
        #[cxx_name = "gridHint"]
        fn grid_hint(self: &KeyMap) -> QString;

        #[qinvokable]
        #[cxx_name = "viewerHint"]
        fn viewer_hint(self: &KeyMap, kind: i32, has_borders: bool) -> QString;

        /// The Yandex panels' action for a key press: a `PanelAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "panelAction"]
        fn panel_action(self: &KeyMap, key: i32) -> i32;
    }
}
