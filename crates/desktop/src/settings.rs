//! What the window remembers. Every preference is `pg_app`'s, in one file (`gallery.json` in the data folder,
//! `PG_PREFS` overrides), so it outlives a catalog rebuild: `app::get().prefs()` reads it and `app::get().prefs().update`
//! changes and saves it. These properties hold no copy: each reads the preferences as they are and writes through them.
//! Nothing goes to the registry or `%LOCALAPPDATA%`, and nothing of ours is kept on C:.

pub(crate) mod state;

use crate::settings_bridge::qobject;

use core::pin::Pin;

use crate::app;
use crate::worker;

/// Change the preferences and save them. The change is in memory at once, and the file is written on the window's
/// writes (`worker`). A save that fails is a note on the app's bus (the window prints it), and the change holds in
/// memory until the app closes.
fn change_prefs(change: impl FnOnce(&mut pg_app::Prefs)) {
    save_prefs(app::get().prefs(), change);
}

/// Changes the preferences: the change is in memory at once, and the file is written on the window's writes (`worker`),
/// off the Qt thread. A failed write is posted as a notice by `PrefSave::write`, so the result needs no handling here.
pub(crate) fn save_prefs(prefs: &pg_app::PrefStore, change: impl FnOnce(&mut pg_app::Prefs)) {
    let save = prefs.change(change);
    worker::run(move || {
        let _ = save.write();
    });
}

impl qobject::Settings {
    pub(crate) fn read_write_back(&self) -> bool {
        app::get().prefs().get().write_back
    }

    pub(crate) fn read_real_size(&self) -> bool {
        app::get().prefs().get().real_size
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

    pub(crate) fn read_detect_borders(&self) -> bool {
        app::get().prefs().get().detect_borders
    }

    pub(crate) fn read_detect_duplicates(&self) -> bool {
        app::get().prefs().get().detect_duplicates
    }

    /// Saves the choice and tells the job, which ends the pass that it turns off (after its batch).
    pub(crate) fn set_detect_borders_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().detect_borders != value {
            change_prefs(|p| p.detect_borders = value);
            app::get().library().set_features(app::get().prefs().get().features());
            self.detect_borders_changed();
        }
    }

    /// Saves the choice and tells the job, which ends the pass that it turns off (after its batch).
    pub(crate) fn set_detect_duplicates_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().detect_duplicates != value {
            change_prefs(|p| p.detect_duplicates = value);
            app::get().library().set_features(app::get().prefs().get().features());
            self.detect_duplicates_changed();
        }
    }

    pub(crate) fn read_thumb_cache(&self) -> bool {
        app::get().prefs().get().thumb_cache
    }

    /// Saves the choice and sets the cache's gate to it at once. The job runs a round: the thumbnail pass resumes from
    /// its checkpoint when the cache is back on, and a round while it is off skips that pass.
    pub(crate) fn set_thumb_cache_saved(self: Pin<&mut Self>, value: bool) {
        if app::get().prefs().get().thumb_cache != value {
            change_prefs(|p| p.thumb_cache = value);
            app::get().thumbs().set_suspended(!value);
            app::get().library().wake_job();
            self.thumb_cache_changed();
        }
    }
}
