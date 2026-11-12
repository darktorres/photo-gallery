//! What the window remembers. Every preference is `pg_app`'s, in one file (`gallery.json` in the data folder,
//! `PG_PREFS` overrides), so it outlives a catalog rebuild: `app::get().prefs()` reads it and `app::get().prefs().update`
//! changes and saves it. These properties hold no copy: each reads the preferences as they are and writes through them.
//! Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing of ours on C:).

pub(crate) mod state;

use crate::settings_bridge::qobject;

use core::pin::Pin;

use pg_model::ids::FileId;

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

    /// The last file's id as QML takes it: -1 for none.
    pub(crate) fn read_last_file(&self) -> i64 {
        app::get().prefs().get().last_file.map_or(-1, FileId::get)
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

    /// `value` is a file's id, or -1 (any negative) for none.
    pub(crate) fn set_last_file_saved(self: Pin<&mut Self>, value: i64) {
        let value = (value >= 0).then(|| FileId::new(value));
        if app::get().prefs().get().last_file != value {
            change_prefs(|p| p.last_file = value);
            self.last_file_changed();
        }
    }
}
