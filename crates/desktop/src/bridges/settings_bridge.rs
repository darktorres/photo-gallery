//! The cxx-qt bridge of [`crate::settings`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::settings_state::SettingsRust;

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
        #[qproperty(bool, write_back, cxx_name = "writeBack", READ = read_write_back, WRITE = set_write_back_saved, NOTIFY, FINAL)]
        /// The viewer opens images at 1:1 instead of fitted (a rating app's Real Size), across images and runs.
        #[qproperty(bool, real_size, cxx_name = "realSize", READ = read_real_size, WRITE = set_real_size_saved, NOTIFY, FINAL)]
        /// Detect the borders of the files, in the background (on by default). Off: the borders pass waits.
        #[qproperty(bool, detect_borders, cxx_name = "detectBorders", READ = read_detect_borders, WRITE = set_detect_borders_saved, NOTIFY, FINAL)]
        /// Detect the duplicates (on by default): the Hashes pass hashes the files and the pairing pass pairs them. Off: both wait.
        #[qproperty(bool, detect_duplicates, cxx_name = "detectDuplicates", READ = read_detect_duplicates, WRITE = set_detect_duplicates_saved, NOTIFY, FINAL)]
        /// Keep the grid's thumbnails in the thumbnail cache (on by default). Off: every thumbnail is decoded from its original.
        #[qproperty(bool, thumb_cache, cxx_name = "thumbCache", READ = read_thumb_cache, WRITE = set_thumb_cache_saved, NOTIFY, FINAL)]
        type Settings = super::SettingsRust;
    }

    unsafe extern "RustQt" {
        fn read_write_back(self: &Settings) -> bool;
        fn read_real_size(self: &Settings) -> bool;
        fn read_detect_borders(self: &Settings) -> bool;
        fn read_detect_duplicates(self: &Settings) -> bool;
        fn read_thumb_cache(self: &Settings) -> bool;
        fn set_write_back_saved(self: Pin<&mut Settings>, value: bool);
        fn set_real_size_saved(self: Pin<&mut Settings>, value: bool);
        fn set_detect_borders_saved(self: Pin<&mut Settings>, value: bool);
        fn set_detect_duplicates_saved(self: Pin<&mut Settings>, value: bool);
        fn set_thumb_cache_saved(self: Pin<&mut Settings>, value: bool);
    }
}
