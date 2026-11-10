//! The cxx-qt bridge of [`crate::trace`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::trace::state::StagesRust;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        /// The stages only the QML sees.
        #[qobject]
        #[qml_element]
        type Stages = super::StagesRust;
    }

    unsafe extern "RustQt" {
        /// The window showed its first frame.
        #[qinvokable]
        #[cxx_name = "firstFrame"]
        fn first_frame(self: &Stages);

        /// The start is done: the list and the tree are in (Main.qml `started`, before it selects the last file
        /// and runs the test hooks).
        #[qinvokable]
        fn ready(self: &Stages);
    }
}
