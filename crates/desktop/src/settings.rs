//! What the window remembers. Every preference is `pg_app`'s, in one file (`gallery.json` in the data folder,
//! `PG_PREFS` overrides), so it outlives a catalog rebuild: `app::get().prefs()` reads it and `app::get().prefs().update`
//! changes and saves it. These properties hold no copy: each reads the preferences as they are and writes through them.
//! Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing of ours on C:).

pub(crate) mod state;

use crate::settings_bridge::qobject;

use core::pin::Pin;

use crate::app;

/// Change the preferences and save them. A save that fails is a note on the app's bus (the window prints it), and the
/// change holds in memory until the app closes.
fn change_prefs(change: impl FnOnce(&mut pg_app::Prefs)) {
    let _ = app::get().prefs().update(change);
}

impl qobject::Settings {
    pub(crate) fn read_write_back(&self) -> bool {
        app::get().prefs().get().write_back
    }

    pub(crate) fn read_real_size(&self) -> bool {
        app::get().prefs().get().real_size
    }

    pub(crate) fn read_show_borders(&self) -> bool {
        app::get().prefs().get().show_borders
    }

    pub(crate) fn read_row_height(&self) -> f64 {
        app::get().prefs().get().row_height
    }

    pub(crate) fn read_last_file(&self) -> i64 {
        app::get().prefs().get().last_file
    }

    pub(crate) fn set_write_back_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().write_back != value {
            change_prefs(|p| p.write_back = value);
            self.write_back_changed();
        }
    }

    pub(crate) fn set_real_size_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().real_size != value {
            change_prefs(|p| p.real_size = value);
            self.real_size_changed();
        }
    }

    pub(crate) fn set_show_borders_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().show_borders != value {
            change_prefs(|p| p.show_borders = value);
            self.show_borders_changed();
        }
    }

    pub(crate) fn set_row_height_saved(self: Pin<&mut Self>, value: f64) {
        if app::get().prefs().get().row_height != value {
            change_prefs(|p| p.row_height = value);
            self.row_height_changed();
        }
    }

    pub(crate) fn set_last_file_saved(self: Pin<&mut Self>, value: i64) {
        if app::get().prefs().get().last_file != value {
            change_prefs(|p| p.last_file = value);
            self.last_file_changed();
        }
    }
}
