//! What the window remembers. The server's preferences (write-back, the folder Yandex results were last saved into) are
//! `pg_app`'s: `app::get().prefs()`, in `gallery.json` beside the catalog folder (`D:\PhotoGallery\gallery.json`;
//! `PG_PREFS` overrides), so they outlive a catalog rebuild. The window's own (Real Size, the borders shown, the row
//! height) are here, in `desktop.json` beside `gallery.json`. The last file selected is the session's `file`, which
//! lives in the catalog with the rest of the session. Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing
//! of ours on C:).

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
use pg_catalog::browse;
use serde::{Deserialize, Serialize};

use crate::app;

/// The window's preferences file's content. Fields missing from an older file take their defaults.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub real_size: bool,
    pub show_borders: bool,
    pub row_height: f64,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { real_size: false, show_borders: false, row_height: 220.0 }
    }
}

static PREFS: Mutex<Option<Prefs>> = Mutex::new(None);

/// `desktop.json`, beside the server's preferences file (so `PG_PREFS` moves both).
fn prefs_path() -> PathBuf {
    pg_app::Config::from_env().prefs_path.with_file_name("desktop.json")
}

/// The window's preferences as they were saved: read from the file the first time.
pub fn prefs() -> Prefs {
    PREFS.lock().unwrap().get_or_insert_with(load).clone()
}

/// Change the window's preferences and save them.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    let mut current = PREFS.lock().unwrap();
    let p = current.get_or_insert_with(load);
    change(p);
    save(p);
}

fn load() -> Prefs {
    let path = prefs_path();
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            // Kept aside, not overwritten by the first save: its content can be copied back by hand.
            let bad = path.with_extension("json.bad");
            let kept = match std::fs::rename(&path, &bad) {
                Ok(()) => format!("is kept as {}", bad.display()),
                Err(r) => format!("could not be kept aside ({r})"),
            };
            crate::panic_log::note(&format!("{} did not parse ({e}); using defaults, and it {kept}", path.display()));
            Prefs::default()
        }),
        // No desktop.json yet: an older version kept these fields in gallery.json, beside the server's. They are taken
        // from there and saved here now, so that a later save of the server's (which rewrites gallery.json without
        // them) can't lose them. The server's load moves a gallery.json it can't parse to gallery.json.bad, so that
        // copy is read too, when it has been moved already.
        Err(_) => {
            let older = ["gallery.json", "gallery.json.bad"]
                .into_iter()
                .find_map(|name| std::fs::read(path.with_file_name(name)).ok());
            match older {
                Some(bytes) => {
                    let p = serde_json::from_slice::<Prefs>(&bytes).unwrap_or_default();
                    save(&p);
                    p
                }
                None => Prefs::default(),
            }
        }
    }
}

/// Written to a temp file and renamed, so a crash never leaves half a file.
fn save(p: &Prefs) {
    let path = prefs_path();
    let temp = path.with_extension("json.tmp");
    let saved = serde_json::to_vec_pretty(p)
        .map_err(anyhow::Error::from)
        .and_then(|json| Ok(std::fs::write(&temp, json)?))
        .and_then(|()| Ok(std::fs::rename(&temp, &path)?));
    if let Err(e) = saved {
        eprintln!("saving {}: {e:#}", path.display());
    }
}

/// Change the server's preferences (`pg_app`'s) and save them.
fn update_server(change: impl FnOnce(&mut pg_app::Prefs)) {
    if let Err(e) = app::get().update_prefs(change) {
        eprintln!("saving the preferences: {e}");
    }
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
        let app = app::get();
        let last_file = app.session_value("file").and_then(|id| id.parse().ok()).unwrap_or(-1);
        SettingsRust {
            write_back: app.prefs().write_back,
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
            update_server(|p| p.write_back = value);
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
            if let Err(e) = app::get().with_catalog(|c| browse::writes::session_set(c, "file", &value.to_string())) {
                eprintln!("session: {e}");
            }
            self.as_mut().rust_mut().last_file = value;
            self.last_file_changed();
        }
    }
}
