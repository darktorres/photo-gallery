//! What the app remembers. Preferences (write-back, Real Size, row height) are the user's, not the catalog's: they
//! go in `gallery.json` beside the catalog folder (`D:\PhotoGallery\gallery.json`; `PG_PREFS` overrides), so they
//! outlive a catalog rebuild. Saved views and where the user was refer to the catalog's folders and files, so they
//! live in it (`views`, `session`). Nothing goes to the registry or `%LOCALAPPDATA%` (PLAN: nothing of ours on C:).

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
        #[qproperty(bool, write_back, cxx_name = "writeBack", READ, WRITE = set_write_back_saved, NOTIFY)]
        /// The viewer opens images at 1:1 instead of fitted (ImageRater's Real Size), across images and runs.
        #[qproperty(bool, real_size, cxx_name = "realSize", READ, WRITE = set_real_size_saved, NOTIFY)]
        /// The grid's row height in logical pixels.
        #[qproperty(f64, row_height, cxx_name = "rowHeight", READ, WRITE = set_row_height_saved, NOTIFY)]
        /// Names of the saved views, sorted.
        #[qproperty(QStringList, views)]
        type Settings = super::SettingsRust;
    }

    unsafe extern "RustQt" {
        fn set_write_back_saved(self: Pin<&mut Settings>, value: bool);
        fn set_real_size_saved(self: Pin<&mut Settings>, value: bool);
        fn set_row_height_saved(self: Pin<&mut Settings>, value: f64);

        /// A saved view: `folder` (merged path, `""` = all), `recursive`, `filter` and `sort` (their keys), or an
        /// empty map. A map inside a QVariant: moc and the QML compiler know that type (not cxx-qt-lib's alias for
        /// the map, which crashed the compiled QML).
        #[qinvokable]
        fn view(self: &Settings, name: &QString) -> QVariant;

        /// Save (or replace) a view.
        #[qinvokable]
        #[cxx_name = "saveView"]
        fn save_view(
            self: Pin<&mut Settings>,
            name: &QString,
            folder: &QString,
            recursive: bool,
            filter: &QString,
            sort: &QString,
        );

        #[qinvokable]
        #[cxx_name = "deleteView"]
        fn delete_view(self: Pin<&mut Settings>, name: &QString);

        /// Where the user was (`folder`, `recursive`, `filter`, `sort`, `file`), `fallback` if unknown.
        #[qinvokable]
        fn session(self: &Settings, key: &QString, fallback: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "setSession"]
        fn set_session(self: &Settings, key: &QString, value: &QString);
    }
}

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::Mutex;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};
use pg_core::browse::{self, Filter, Sort, View};
use serde::{Deserialize, Serialize};

use crate::state;

/// The preferences file's content. Fields missing from an older file take their defaults.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub write_back: bool,
    pub real_size: bool,
    pub row_height: f64,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { write_back: true, real_size: false, row_height: 220.0 }
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
                eprintln!("{}: {e}; using defaults", path.display());
                Prefs::default()
            }),
            Err(_) => Prefs::default(),
        }
    })
    .clone()
}

/// Change the preferences and save them. Written to a temp file and renamed, so a crash never leaves half a file.
fn update(change: impl FnOnce(&mut Prefs)) {
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
    row_height: f64,
    views: QStringList,
}

impl Default for SettingsRust {
    fn default() -> SettingsRust {
        let p = prefs();
        SettingsRust { write_back: p.write_back, real_size: p.real_size, row_height: p.row_height, views: view_names() }
    }
}

fn view_names() -> QStringList {
    state::with_catalog(|c| browse::views(&c.conn))
        .unwrap_or_else(|e| {
            eprintln!("views: {e:#}");
            Vec::new()
        })
        .iter()
        .map(|v| QString::from(&v.name))
        .collect()
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

    fn set_row_height_saved(mut self: Pin<&mut Self>, value: f64) {
        if self.row_height != value {
            update(|p| p.row_height = value);
            self.as_mut().rust_mut().row_height = value;
            self.row_height_changed();
        }
    }

    fn view(&self, name: &QString) -> QVariant {
        let name = name.to_string();
        let mut map = QMap::<QMapPair_QString_QVariant>::default();
        let found =
            state::with_catalog(|c| browse::views(&c.conn)).ok().and_then(|vs| vs.into_iter().find(|v| v.name == name));
        if let Some(v) = found {
            map.insert(QString::from("folder"), QVariant::from(&QString::from(&v.folder)));
            map.insert(QString::from("recursive"), QVariant::from(&v.recursive));
            map.insert(QString::from("filter"), QVariant::from(&QString::from(&v.filter.key())));
            map.insert(QString::from("sort"), QVariant::from(&QString::from(v.sort.key())));
        }
        QVariant::from(&map)
    }

    fn save_view(
        self: Pin<&mut Self>,
        name: &QString,
        folder: &QString,
        recursive: bool,
        filter: &QString,
        sort: &QString,
    ) {
        let name = name.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let view = View {
            name,
            folder: folder.to_string(),
            recursive,
            filter: Filter::from_key(&filter.to_string()),
            sort: Sort::from_key(&sort.to_string()),
        };
        if let Err(e) = state::with_catalog(|c| browse::save_view(&c.conn, &view)) {
            eprintln!("saving view: {e:#}");
        }
        self.set_views(view_names());
    }

    fn delete_view(self: Pin<&mut Self>, name: &QString) {
        if let Err(e) = state::with_catalog(|c| browse::delete_view(&c.conn, &name.to_string())) {
            eprintln!("deleting view: {e:#}");
        }
        self.set_views(view_names());
    }

    fn session(&self, key: &QString, fallback: &QString) -> QString {
        match state::with_catalog(|c| browse::session_get(&c.conn, &key.to_string())) {
            Ok(Some(v)) => QString::from(&v),
            _ => fallback.clone(),
        }
    }

    fn set_session(&self, key: &QString, value: &QString) {
        if let Err(e) = state::with_catalog(|c| browse::session_set(&c.conn, &key.to_string(), &value.to_string())) {
            eprintln!("session: {e:#}");
        }
    }
}
