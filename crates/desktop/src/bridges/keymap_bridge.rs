//! The cxx-qt bridge of [`crate::keymap`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::keymap_state::KeyMapRust;

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

        /// The grid's key hint line. `detection`: border detection is on (the B and N keys are named only then).
        #[qinvokable]
        #[cxx_name = "gridHint"]
        fn grid_hint(self: &KeyMap, detection: bool) -> QString;

        /// The viewer's key hint as chips, indexed from 0 in the line's order: how many there are, then the key of each
        /// and the label after it ("" for a key alone). `kind` and `has_borders` as in `viewerAction`.
        #[qinvokable]
        #[cxx_name = "viewerHintCount"]
        fn viewer_hint_count(self: &KeyMap, kind: i32, has_borders: bool) -> i32;

        #[qinvokable]
        #[cxx_name = "viewerHintKey"]
        fn viewer_hint_key(self: &KeyMap, kind: i32, has_borders: bool, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "viewerHintLabel"]
        fn viewer_hint_label(self: &KeyMap, kind: i32, has_borders: bool, index: i32) -> QString;

        /// The key sequence of a window shortcut (a `GlobalAction` code), for its `Shortcut`; empty for no code.
        #[qinvokable]
        #[cxx_name = "globalSequence"]
        fn global_sequence(self: &KeyMap, code: i32) -> QString;

        /// The Yandex panels' action for a key press: a `PanelAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "panelAction"]
        fn panel_action(self: &KeyMap, key: i32) -> i32;
    }
}
