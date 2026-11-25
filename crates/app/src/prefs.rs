//! The preferences (`gallery.json`): every choice the app remembers across runs, the front ends' included, saved atomically
//! beside the catalog folder (so they outlive a catalog rebuild). The window's Real Size, the borders shown and the row
//! height are here with the library's write-back and the Yandex folder, and dev mode. The last file selected is here too: it is a
//! file's id in the catalog, so after a rebuild it may name another file (or none).

use std::path::PathBuf;
use std::sync::Arc;

use pg_jobs::NoticeSink;
use pg_model::features::Features;
use pg_model::ids::FileStamp;
use pg_model::rules::Path as RulePath;
use pg_model::size::RowHeight;
use pg_model::versions::PREFS_VERSION;
use serde::{Deserialize, Serialize};

use crate::store::{JsonStore, Save};
use pg_model::notice::{ERROR_CAP, Notice, Trigger, tidy};

/// The preferences file's content. Fields missing from a file take their defaults; fields the file has that this version
/// doesn't know are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The layout the file was written in (`PREFS_VERSION`). Every save stamps it; a file without the key reads as 0 (the
    /// field-level default, not the struct's), so the layout it was written in stays known until it is saved again.
    #[serde(default)]
    pub version: i64,
    /// Write ratings and turns into the files too (on by default); off keeps them in the catalog only.
    pub write_back: bool,
    /// The folder Yandex results were last saved into; None before the first save.
    #[serde(with = "yandex_folder")]
    pub yandex_folder: Option<RulePath>,
    /// The viewer opens images at 1:1 instead of fitted (a rating app's Real Size), across images and runs.
    pub real_size: bool,
    /// The grid and the viewer show what cropping the borders would cut (B).
    pub show_borders: bool,
    /// The grid's row height in logical pixels.
    #[serde(with = "row_height")]
    pub row_height: RowHeight,
    /// The file selected (or open in the viewer) last, if any.
    #[serde(with = "last_file")]
    pub last_file: Option<FileStamp>,
    /// Dev mode: the window's log view (Ctrl+Shift+D turns it on and off). A start's `--dev=1` or `PG_DEV=1` turns it on
    /// for that run without saving it.
    pub dev_mode: bool,
    /// Detect the borders of the files, in the background (on by default). Off: the pass waits; what it found stays.
    pub detect_borders: bool,
    /// Detect the duplicates (on by default): the import hashes the files, and the pairing pass pairs them. Off: the
    /// import leaves the hashes out (the Hashes pass makes them when it is back on), and the pairing waits.
    pub detect_duplicates: bool,
    /// Keep the grid's thumbnails in the thumbnail cache (on by default). Off: nothing is kept or read from it, so every
    /// thumbnail is decoded from its original each time it is shown, the background thumbnail pass waits, and the
    /// Placeholders pass makes no new ThumbHashes (the stored ones still show). What the cache holds is kept, and the
    /// setting takes effect at once.
    pub thumb_cache: bool,
}

impl Prefs {
    /// The background features these preferences turn on (`pg_model::features`).
    pub fn features(&self) -> Features {
        Features { borders: self.detect_borders, duplicates: self.detect_duplicates }
    }
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

/// `row_height` as the file holds it: a whole number of pixels. A file written before it was whole reads its float, rounded
/// and clamped into the grid's range, so an old value is kept as the nearest one the grid takes. This module is the file
/// boundary for a row height: the one place its number is converted to and from `RowHeight`. Everything else in the app
/// takes the typed value.
mod row_height {
    use pg_model::size::RowHeight;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<S: Serializer>(row: &RowHeight, serializer: S) -> Result<S::Ok, S::Error> {
        row.get().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RowHeight, D::Error> {
        let px = f64::deserialize(deserializer)?;
        RowHeight::from_finite(px).ok_or_else(|| serde::de::Error::custom(format!("row height {px} is not a number")))
    }
}

/// `last_file` as the file holds it: the file's stamp (its identity, size and mtime), or `null` for none. A number
/// reads as none, whatever it is: a row id (or the old `-1`) means something only in the catalog that assigned it, and
/// resolving it would select whichever file holds that id now.
mod last_file {
    use pg_model::ids::{FileIdentity, FileStamp};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Stored {
        volume: u64,
        file_index: u64,
        size: i64,
        mtime: i64,
    }

    /// What the file may hold: a stamp, or anything else, which is read as none (the legacy numbers among it).
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Read {
        Stamp(Stored),
        Other(serde::de::IgnoredAny),
    }

    fn stored(stamp: FileStamp) -> Stored {
        Stored {
            volume: stamp.identity.volume,
            file_index: stamp.identity.file_index,
            size: stamp.size,
            mtime: stamp.mtime,
        }
    }

    pub(crate) fn serialize<S: Serializer>(file: &Option<FileStamp>, serializer: S) -> Result<S::Ok, S::Error> {
        file.map(stored).serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<FileStamp>, D::Error> {
        Ok(match Option::<Read>::deserialize(deserializer)? {
            Some(Read::Stamp(s)) => {
                Some(FileStamp { identity: FileIdentity::new(s.volume, s.file_index), size: s.size, mtime: s.mtime })
            }
            Some(Read::Other(_)) | None => None,
        })
    }
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            version: PREFS_VERSION,
            write_back: true,
            yandex_folder: None,
            real_size: false,
            show_borders: false,
            row_height: RowHeight::DEFAULT,
            last_file: None,
            dev_mode: false,
            detect_borders: true,
            detect_duplicates: true,
            thumb_cache: true,
        }
    }
}

/// The preferences in memory and in their file. A change is saved when it is made, atomically (`JsonStore`). A change that
/// can't be saved holds in memory, and a notice says so (until a save goes through).
pub struct PrefStore {
    store: JsonStore<Prefs>,
    path: PathBuf,
    inbox: Arc<dyn NoticeSink>,
}

impl PrefStore {
    /// Read the preferences at `path`: the defaults where the file is missing or doesn't parse (a notice says why, and where
    /// the unreadable file was kept).
    pub(crate) fn open(path: PathBuf, inbox: Arc<dyn NoticeSink>) -> PrefStore {
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
        self.change(change).write()
    }

    /// Change the preferences now, and return the save of the file: `PrefSave::write` writes it, on any thread, so the
    /// disk is not waited for where the change is made.
    pub fn change(&self, change: impl FnOnce(&mut Prefs)) -> PrefSave {
        let save = self.store.change(|prefs| {
            change(prefs);
            prefs.version = PREFS_VERSION;
        });
        PrefSave { save, path: self.path.clone(), inbox: self.inbox.clone() }
    }
}

/// A change to the preferences, whose file is not written yet (`PrefStore::change`). `write` writes it and reports the
/// outcome as `PrefStore::update` does.
pub struct PrefSave {
    save: Save<Prefs>,
    path: PathBuf,
    inbox: Arc<dyn NoticeSink>,
}

impl PrefSave {
    /// Write the preferences with this change. A failure is posted as a notice; a success clears the notice an earlier
    /// failure kept.
    pub fn write(self) -> std::io::Result<()> {
        let result = self.save.write();
        match &result {
            Ok(()) => self.inbox.resolve(Trigger::Save),
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
  "version": 1,
  "write_back": true,
  "yandex_folder": "",
  "real_size": false,
  "show_borders": false,
  "row_height": 220,
  "last_file": null,
  "dev_mode": false,
  "detect_borders": true,
  "detect_duplicates": true,
  "thumb_cache": true
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
    fn a_file_without_a_version_reads_as_version_zero() {
        assert_eq!(serde_json::from_str::<Prefs>(r#"{"row_height": 333.5}"#).unwrap().version, 0);
        assert_eq!(serde_json::from_str::<Prefs>(GOLDEN).unwrap().version, PREFS_VERSION);
    }

    #[test]
    fn a_float_row_height_from_an_old_file_reads_rounded_and_clamped() {
        let read =
            |px: &str| serde_json::from_str::<Prefs>(&format!(r#"{{"row_height": {px}}}"#)).unwrap().row_height.get();
        assert_eq!(read("333.5"), 334);
        assert_eq!(read("5000.0"), 800);
        assert_eq!(read("-3.0"), 60);
    }

    #[test]
    fn a_changed_value_is_written_as_its_own_text() {
        let prefs = Prefs {
            real_size: true,
            row_height: RowHeight::new(334).expect("a row height in range"),
            last_file: Some(stamp()),
            dev_mode: true,
            ..Prefs::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        assert_eq!(
            text,
            r#"{"version":1,"write_back":true,"yandex_folder":"","real_size":true,"show_borders":false,"row_height":334,"last_file":{"volume":7,"file_index":42,"size":10,"mtime":20},"dev_mode":true,"detect_borders":true,"detect_duplicates":true,"thumb_cache":true}"#
        );
    }

    fn stamp() -> FileStamp {
        FileStamp { identity: pg_model::ids::FileIdentity::new(7, 42), size: 10, mtime: 20 }
    }

    // A stamp round-trips; `null` is none; a number reads as none, whatever it is (a row id, or the old `-1`).
    #[test]
    fn the_last_file_is_its_stamp_and_a_number_reads_as_none() {
        let with = |value: &str| -> Option<FileStamp> {
            serde_json::from_str::<Prefs>(&GOLDEN.replace("\"last_file\": null", &format!("\"last_file\": {value}")))
                .unwrap()
                .last_file
        };
        let saved = r#"{"volume":7,"file_index":42,"size":10,"mtime":20}"#;
        assert_eq!(with(saved), Some(stamp()));
        assert_eq!(with("null"), None);
        assert_eq!(with("-1"), None);
        assert_eq!(with("42"), None, "a row id is not resolved");
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
