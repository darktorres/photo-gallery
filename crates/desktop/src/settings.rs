//! What the app remembers. Preferences (write-back, Real Size, borders shown, row height, the folder Yandex results
//! were last saved into) are the user's, not the catalog's: they go in `gallery.json` beside the catalog folder
//! (`D:\PhotoGallery\gallery.json`; `PG_PREFS` overrides), so they outlive a catalog rebuild. Saved views and where
//! the user was refer to the catalog's folders and files, so they live in it (`views`, `session`): the views and the
//! session's folder, query and sort are `view::View`'s, the
//! selected file is here. Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing of ours on C:).

#[cxx_qt::bridge]
pub mod qobject {
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
        #[qproperty(bool, write_back, cxx_name = "writeBack", READ, WRITE = set_write_back_saved, NOTIFY, FINAL)]
        /// The viewer opens images at 1:1 instead of fitted (ImageRater's Real Size), across images and runs.
        #[qproperty(bool, real_size, cxx_name = "realSize", READ, WRITE = set_real_size_saved, NOTIFY, FINAL)]
        /// The grid and the viewer show what cropping the borders would cut (B).
        #[qproperty(bool, show_borders, cxx_name = "showBorders", READ, WRITE = set_show_borders_saved, NOTIFY, FINAL)]
        /// The grid's row height in logical pixels.
        #[qproperty(f64, row_height, cxx_name = "rowHeight", READ, WRITE = set_row_height_saved, NOTIFY, FINAL)]
        /// The id of the file selected (or open in the viewer) last, -1 for none: the session's `file`.
        #[qproperty(i64, last_file, cxx_name = "lastFile", READ, WRITE = set_last_file_saved, NOTIFY, FINAL)]
        type Settings = super::SettingsRust;
    }

    unsafe extern "RustQt" {
        fn set_write_back_saved(self: Pin<&mut Settings>, value: bool);
        fn set_real_size_saved(self: Pin<&mut Settings>, value: bool);
        fn set_show_borders_saved(self: Pin<&mut Settings>, value: bool);
        fn set_row_height_saved(self: Pin<&mut Settings>, value: f64);
        fn set_last_file_saved(self: Pin<&mut Settings>, value: i64);
    }
}

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::Mutex;

use cxx_qt::CxxQtType;
use pg_core::browse;
use serde::{Deserialize, Serialize};

use crate::state;

/// The preferences file's content. Fields missing from an older file take their defaults.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub write_back: bool,
    pub real_size: bool,
    pub show_borders: bool,
    pub row_height: f64,
    /// The folder Yandex results were last saved into, empty before the first save.
    pub yandex_folder: String,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            write_back: true,
            real_size: false,
            show_borders: false,
            row_height: 220.0,
            yandex_folder: String::new(),
        }
    }
}

static PREFS: Mutex<Option<Prefs>> = Mutex::new(None);

fn prefs_path() -> PathBuf {
    std::env::var_os("PG_PREFS").map_or_else(
        || {
            let dir = &state::get().catalog_dir;
            dir.parent().unwrap_or(dir).join("gallery.json")
        },
        PathBuf::from,
    )
}

/// The current preferences (read from the file the first time).
pub fn prefs() -> Prefs {
    let mut p = PREFS.lock().unwrap();
    p.get_or_insert_with(|| {
        let path = prefs_path();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // Kept aside, not overwritten by the first save: its content (the Yandex folder, the
                // write-back setting) can be copied back by hand.
                let bad = path.with_extension("json.bad");
                let kept = match std::fs::rename(&path, &bad) {
                    Ok(()) => format!("is kept as {}", bad.display()),
                    Err(r) => format!("could not be kept aside ({r})"),
                };
                crate::panic_log::note(&format!(
                    "{} did not parse ({e}); using defaults, and it {kept}",
                    path.display()
                ));
                Prefs::default()
            }),
            Err(_) => Prefs::default(),
        }
    })
    .clone()
}

/// Change the preferences and save them. Written to a temp file and renamed, so a crash never leaves half a file.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    let mut p = prefs();
    change(&mut p);
    let path = prefs_path();
    let temp = path.with_extension("json.tmp");
    let saved = serde_json::to_vec_pretty(&p)
        .map_err(anyhow::Error::from)
        .and_then(|json| Ok(std::fs::write(&temp, json)?))
        .and_then(|()| Ok(std::fs::rename(&temp, &path)?));
    if let Err(e) = saved {
        eprintln!("saving {}: {e:#}", path.display());
    }
    *PREFS.lock().unwrap() = Some(p);
}

pub struct SettingsRust {
    write_back: bool,
    real_size: bool,
    show_borders: bool,
    row_height: f64,
    last_file: i64,
}

impl Default for SettingsRust {
    fn default() -> SettingsRust {
        let p = prefs();
        let last_file = state::with_catalog(|c| browse::read::session_get(&c.conn, "file"))
            .ok()
            .flatten()
            .and_then(|id| id.parse().ok())
            .unwrap_or(-1);
        SettingsRust {
            write_back: p.write_back,
            real_size: p.real_size,
            show_borders: p.show_borders,
            row_height: p.row_height,
            last_file,
        }
    }
}

impl qobject::Settings {
    fn set_write_back_saved(mut self: Pin<&mut Self>, value: bool) {
        if self.write_back != value {
            update(|p| p.write_back = value);
            self.as_mut().rust_mut().write_back = value;
            self.write_back_changed();
        }
    }

    fn set_real_size_saved(mut self: Pin<&mut Self>, value: bool) {
        if self.real_size != value {
            update(|p| p.real_size = value);
            self.as_mut().rust_mut().real_size = value;
            self.real_size_changed();
        }
    }

    fn set_show_borders_saved(mut self: Pin<&mut Self>, value: bool) {
        if self.show_borders != value {
            update(|p| p.show_borders = value);
            self.as_mut().rust_mut().show_borders = value;
            self.show_borders_changed();
        }
    }

    fn set_row_height_saved(mut self: Pin<&mut Self>, value: f64) {
        if self.row_height != value {
            update(|p| p.row_height = value);
            self.as_mut().rust_mut().row_height = value;
            self.row_height_changed();
        }
    }

    fn set_last_file_saved(mut self: Pin<&mut Self>, value: i64) {
        if self.last_file != value {
            if let Err(e) = state::with_catalog(|c| browse::writes::session_set(&c.conn, "file", &value.to_string())) {
                eprintln!("session: {e:#}");
            }
            self.as_mut().rust_mut().last_file = value;
            self.last_file_changed();
        }
    }
}
