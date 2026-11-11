//! The preferences (`gallery.json`): every choice the app remembers across runs, the front ends' included, saved atomically
//! beside the catalog folder (so they outlive a catalog rebuild). The window's Real Size, the borders shown and the row
//! height are here with the library's write-back and the Yandex folder, and dev mode. The last file selected is here too: it is a
//! file's id in the catalog, so after a rebuild it may name another file (or none).

use std::path::PathBuf;
use std::sync::Arc;

use pg_model::ids::FileId;
use pg_model::rules::Path as RulePath;
use serde::{Deserialize, Serialize};

use crate::inbox::Inbox;
use crate::store::JsonStore;
use pg_model::notice::{ERROR_CAP, Key, Notice, tidy};

/// The preferences file's content. Fields missing from a file take their defaults; fields the file has that this version
/// doesn't know are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
    pub write_back: bool,
    /// The folder Yandex results were last saved into; None before the first save.
    #[serde(with = "yandex_folder")]
    pub yandex_folder: Option<RulePath>,
    /// The viewer opens images at 1:1 instead of fitted (ImageRater's Real Size), across images and runs.
    pub real_size: bool,
    /// The grid and the viewer show what cropping the borders would cut (B).
    pub show_borders: bool,
    /// The grid's row height in logical pixels.
    pub row_height: f64,
    /// The file selected (or open in the viewer) last, if any.
    #[serde(with = "last_file")]
    pub last_file: Option<FileId>,
    /// Dev mode: the window's log view (Ctrl+Shift+D turns it on and off). A start's `--dev=1` or `PG_DEV=1` turns it on
    /// for that run without saving it.
    pub dev_mode: bool,
}

/// `yandex_folder` as the file holds it: the folder's path as a name (`Name`: its text when that is UTF-8, `{"b": hex}`
/// otherwise), and `""` for none. Read back under the native rules, the path it was written from. A text that is no path
/// reads as none.
mod yandex_folder {
    use pg_model::name::Name;
    use pg_model::rules::Path as RulePath;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<S: Serializer>(folder: &Option<RulePath>, serializer: S) -> Result<S::Ok, S::Error> {
        match folder {
            None => serializer.serialize_str(""),
            Some(path) => Name::from_raw(&pg_platform::native::PATH_RULES.join(path)).serialize(serializer),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<RulePath>, D::Error> {
        let name = Name::deserialize(deserializer)?;
        if name.as_bytes().is_empty() {
            return Ok(None);
        }
        Ok(pg_platform::native::PATH_RULES.parse_bytes(name.as_bytes()))
    }
}

/// `last_file` as the file holds it: the file's id, or `-1` for none. A negative number reads as none.
mod last_file {
    use pg_model::ids::FileId;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(file: &Option<FileId>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(file.map_or(-1, FileId::get))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<FileId>, D::Error> {
        let id = i64::deserialize(deserializer)?;
        Ok((id >= 0).then(|| FileId::new(id)))
    }
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            write_back: true,
            yandex_folder: None,
            real_size: false,
            show_borders: false,
            row_height: 220.0,
            last_file: None,
            dev_mode: false,
        }
    }
}

/// The preferences in memory and in their file. A change is saved when it is made, atomically (`JsonStore`). A change that
/// can't be saved holds in memory, and a notice says so (until a save goes through).
pub struct PrefStore {
    store: JsonStore<Prefs>,
    path: PathBuf,
    inbox: Arc<Inbox>,
}

impl PrefStore {
    /// Read the preferences at `path`: the defaults where the file is missing or doesn't parse (a notice says why, and where
    /// the unreadable file was kept).
    pub(crate) fn open(path: PathBuf, inbox: Arc<Inbox>) -> PrefStore {
        let (store, unreadable) = JsonStore::open(path.clone());
        if let Some(u) = unreadable {
            let (kept, keep_error) = match u.kept {
                Ok(kept) => (Some(kept.display().to_string()), None),
                Err(e) => (None, Some(tidy(&e, ERROR_CAP))),
            };
            inbox.post(Notice::PrefsUnreadable {
                path: path.display().to_string(),
                error: tidy(&u.error, ERROR_CAP),
                kept,
                keep_error,
            });
        }
        PrefStore { store, path, inbox }
    }

    /// The preferences, as they are now.
    pub fn get(&self) -> Prefs {
        self.store.get()
    }

    /// Change the preferences and save them. The change is kept in memory even when the save fails; the error says why
    /// it wasn't written.
    pub fn update(&self, change: impl FnOnce(&mut Prefs)) -> std::io::Result<()> {
        let result = self.store.update(change);
        match &result {
            Ok(()) => self.inbox.clear(Key::all("prefs-not-saved", None).to_buf()),
            Err(e) => self.inbox.post(Notice::PrefsNotSaved {
                path: self.path.display().to_string(),
                error: tidy(&e.to_string(), ERROR_CAP),
            }),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's text, pinned: the field names, their order and the number formats are the format on disk.
    const GOLDEN: &str = r#"{
  "write_back": true,
  "yandex_folder": "",
  "real_size": false,
  "show_borders": false,
  "row_height": 220.0,
  "last_file": -1,
  "dev_mode": false
}"#;

    #[test]
    fn the_defaults_are_written_as_the_golden_text() {
        assert_eq!(serde_json::to_string_pretty(&Prefs::default()).unwrap(), GOLDEN);
    }

    #[test]
    fn the_golden_text_reads_back_to_the_defaults() {
        assert_eq!(serde_json::from_str::<Prefs>(GOLDEN).unwrap(), Prefs::default());
    }

    #[test]
    fn a_changed_value_is_written_as_its_own_text() {
        let prefs = Prefs {
            real_size: true,
            row_height: 333.5,
            last_file: Some(FileId::new(42)),
            dev_mode: true,
            ..Prefs::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(
            text,
            r#"{"write_back":true,"yandex_folder":"","real_size":true,"show_borders":false,"row_height":333.5,"last_file":42,"dev_mode":true}"#
        );
    }

    /// A folder whose name is no UTF-8: a lone surrogate (its WTF-8 bytes) on Windows, a raw `0xFF` byte on Unix.
    #[cfg(windows)]
    const ODD_FOLDER: &[u8] = b"C:\\photos\\\xED\xA0\x80";
    /// `ODD_FOLDER`'s bytes in hex, as `Name` writes a name that is no UTF-8.
    #[cfg(windows)]
    const ODD_HEX: &str = "433a5c70686f746f735ceda080";
    #[cfg(not(windows))]
    const ODD_FOLDER: &[u8] = b"/photos/\xFF";
    #[cfg(not(windows))]
    const ODD_HEX: &str = "2f70686f746f732fff";

    /// A folder whose name is UTF-8: it is written as its text.
    #[cfg(windows)]
    const TEXT_FOLDER: &[u8] = br"C:\pics";
    #[cfg(not(windows))]
    const TEXT_FOLDER: &[u8] = b"/pics";

    /// The text an older version wrote for a folder, and the folder it names.
    #[cfg(windows)]
    const OLD_TEXT: &str = r#"{"yandex_folder":"D:\\x"}"#;
    #[cfg(windows)]
    const OLD_PATH: &[u8] = br"D:\x";
    #[cfg(not(windows))]
    const OLD_TEXT: &str = r#"{"yandex_folder":"/x"}"#;
    #[cfg(not(windows))]
    const OLD_PATH: &[u8] = b"/x";

    #[test]
    fn a_folder_that_is_no_utf8_is_written_as_hex_and_read_back() {
        let folder = pg_platform::native::PATH_RULES.parse_bytes(ODD_FOLDER).expect("a path");
        let prefs = Prefs { yandex_folder: Some(folder.clone()), ..Prefs::default() };
        assert_eq!(serde_json::to_value(&prefs).unwrap()["yandex_folder"], serde_json::json!({ "b": ODD_HEX }));
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap().yandex_folder, Some(folder));
    }

    #[test]
    fn a_folder_that_is_utf8_is_written_as_its_text_and_read_back() {
        let folder = pg_platform::native::PATH_RULES.parse_bytes(TEXT_FOLDER).expect("a path");
        let prefs = Prefs { yandex_folder: Some(folder.clone()), ..Prefs::default() };
        let text_form = std::str::from_utf8(TEXT_FOLDER).unwrap();
        assert_eq!(serde_json::to_value(&prefs).unwrap()["yandex_folder"], text_form);
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap().yandex_folder, Some(folder));
    }

    #[test]
    fn the_text_an_older_version_wrote_reads_as_the_folder_it_named() {
        let read = serde_json::from_str::<Prefs>(OLD_TEXT).unwrap();
        assert_eq!(read.yandex_folder, pg_platform::native::PATH_RULES.parse_bytes(OLD_PATH));
    }
}
