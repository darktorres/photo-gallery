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
        /// The grid and the viewer show what cropping the borders would cut (B).
        #[qproperty(bool, show_borders, cxx_name = "showBorders", READ = read_show_borders, WRITE = set_show_borders_saved, NOTIFY, FINAL)]
        /// The grid's row height in logical pixels.
        #[qproperty(f64, row_height, cxx_name = "rowHeight", READ = read_row_height, WRITE = set_row_height_saved, NOTIFY, FINAL)]
        /// The id of the file selected (or open in the viewer) last, -1 for none.
        #[qproperty(i64, last_file, cxx_name = "lastFile", READ = read_last_file, WRITE = set_last_file_saved, NOTIFY, FINAL)]
        type Settings = super::SettingsRust;
    }

    unsafe extern "RustQt" {
        fn read_write_back(self: &Settings) -> bool;
        fn read_real_size(self: &Settings) -> bool;
        fn read_show_borders(self: &Settings) -> bool;
        fn read_row_height(self: &Settings) -> f64;
        fn read_last_file(self: &Settings) -> i64;

        fn set_write_back_saved(self: Pin<&mut Settings>, value: bool);
        fn set_real_size_saved(self: Pin<&mut Settings>, value: bool);
        fn set_show_borders_saved(self: Pin<&mut Settings>, value: bool);
        fn set_row_height_saved(self: Pin<&mut Settings>, value: f64);
        fn set_last_file_saved(self: Pin<&mut Settings>, value: i64);
    }
}
