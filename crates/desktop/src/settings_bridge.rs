//! The cxx-qt bridge of [`crate::settings`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::settings::state::SettingsRust;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
        #[qproperty(bool, write_back, cxx_name = "writeBack", READ = read_write_back, WRITE = set_write_back_saved, NOTIFY, FINAL)]
        /// The viewer opens images at 1:1 instead of fitted (ImageRater's Real Size), across images and runs.
        #[qproperty(bool, real_size, cxx_name = "realSize", READ = read_real_size, WRITE = set_real_size_saved, NOTIFY, FINAL)]
        type Settings = super::SettingsRust;
    }

    unsafe extern "RustQt" {
        fn read_write_back(self: &Settings) -> bool;
        fn read_real_size(self: &Settings) -> bool;
        fn set_write_back_saved(self: Pin<&mut Settings>, value: bool);
        fn set_real_size_saved(self: Pin<&mut Settings>, value: bool);
    }
}
