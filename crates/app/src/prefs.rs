//! The server's preferences (`gallery.json`): the choices about the library as a whole, saved atomically. The client's
//! (the window's Real Size, the borders shown, the row height) belong to each front end and are not here.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app::Note;

/// The preferences file's content. Fields missing from an older file take their defaults; fields this version doesn't
/// know (a front end's, from before the split) are ignored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
    pub write_back: bool,
    /// The folder Yandex results were last saved into, empty before the first save.
    pub yandex_folder: String,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { write_back: true, yandex_folder: String::new() }
    }
}

pub(crate) struct Store {
    path: PathBuf,
    current: Mutex<Prefs>,
}

impl Store {
    /// Read the file (the defaults when there is none). One that doesn't parse is kept aside, not overwritten by the
    /// next save: its content (the Yandex folder, the write-back setting) can be copied back by hand. The note says so.
    pub(crate) fn load(path: PathBuf) -> (Store, Option<Note>) {
        let (prefs, note) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Prefs>(&bytes) {
                Ok(prefs) => (prefs, None),
                Err(e) => {
                    let bad = path.with_extension("json.bad");
                    let kept = std::fs::rename(&path, &bad).map(|()| bad).map_err(|r| r.to_string());
                    (Prefs::default(), Some(Note::PrefsUnreadable { path: path.clone(), error: e.to_string(), kept }))
                }
            },
            Err(_) => (Prefs::default(), None),
        };
        (Store { path, current: Mutex::new(prefs) }, note)
    }

    pub(crate) fn get(&self) -> Prefs {
        self.current.lock().unwrap().clone()
    }

    /// Change the preferences and save them, written to a temp file and renamed, so a crash never leaves half a file. The
    /// change is kept in memory even when the save fails; the error says why it wasn't written.
    pub(crate) fn update(&self, change: impl FnOnce(&mut Prefs)) -> std::io::Result<()> {
        let mut current = self.current.lock().unwrap();
        change(&mut current);
        let json = serde_json::to_vec_pretty(&*current).map_err(std::io::Error::other)?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, json).and_then(|()| std::fs::rename(&temp, &self.path))
    }
}
