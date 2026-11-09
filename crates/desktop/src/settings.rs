//! What the window remembers. Every preference is `pg_app`'s, in one file (`gallery.json` in the data folder,
//! `PG_PREFS` overrides), so it outlives a catalog rebuild: `app::get().prefs()` reads it and `app::get().update_prefs`
//! changes and saves it. These properties hold no copy: each reads the preferences as they are and writes through them.
//! Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing of ours on C:).

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

use core::pin::Pin;

use crate::app;

/// The properties' state is the preferences': the struct holds nothing.
#[derive(Default)]
pub struct SettingsRust;

/// Change the preferences and save them. A save that fails is a note on the app's bus (the window prints it), and the
/// change holds in memory until the app closes.
fn change_prefs(change: impl FnOnce(&mut pg_app::Prefs)) {
    let _ = app::get().update_prefs(change);
}

impl qobject::Settings {
    fn read_write_back(&self) -> bool {
        app::get().prefs().write_back
    }

    fn read_real_size(&self) -> bool {
        app::get().prefs().real_size
    }

    fn read_show_borders(&self) -> bool {
        app::get().prefs().show_borders
    }

    fn read_row_height(&self) -> f64 {
        app::get().prefs().row_height
    }

    fn read_last_file(&self) -> i64 {
        app::get().prefs().last_file
    }

    fn set_write_back_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().write_back != value {
            change_prefs(|p| p.write_back = value);
            self.write_back_changed();
        }
    }

    fn set_real_size_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().real_size != value {
            change_prefs(|p| p.real_size = value);
            self.real_size_changed();
        }
    }

    fn set_show_borders_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().show_borders != value {
            change_prefs(|p| p.show_borders = value);
            self.show_borders_changed();
        }
    }

    fn set_row_height_saved(self: Pin<&mut Self>, value: f64) {
        if app::get().prefs().row_height != value {
            change_prefs(|p| p.row_height = value);
            self.row_height_changed();
        }
    }

    fn set_last_file_saved(self: Pin<&mut Self>, value: i64) {
        if app::get().prefs().last_file != value {
            change_prefs(|p| p.last_file = value);
            self.last_file_changed();
        }
    }
}
