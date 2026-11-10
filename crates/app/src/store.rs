//! A JSON file the app keeps a value in: read once when it is opened, changed in memory, and written back atomically on
//! each change. Used for the preferences; the path is given at construction and resolved by the owner, not again here.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use pg_platform::file::beside;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A file that didn't read. It was moved aside to `kept` (or couldn't be, with the reason as the `Err`), and the defaults
/// are in use. The file is kept, not overwritten by the next save: its content can be copied back by hand.
#[derive(Clone, Debug)]
pub struct Unreadable {
    pub error: String,
    pub kept: Result<PathBuf, String>,
}

pub struct JsonStore<T> {
    path: PathBuf,
    current: Mutex<T>,
}

impl<T: Serialize + DeserializeOwned + Default> JsonStore<T> {
    /// Read the file. A missing file gives the defaults. One that can't be read or parsed is quarantined (moved to
    /// `<name>.bad`, or the next free `<name>.bad.N`) and the defaults are used; the returned [`Unreadable`] says so.
    pub fn open(path: PathBuf) -> (JsonStore<T>, Option<Unreadable>) {
        let (value, unreadable) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
                Ok(value) => (value, None),
                Err(e) => (T::default(), Some(quarantine(&path, e.to_string()))),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (T::default(), None),
            Err(e) => (T::default(), Some(quarantine(&path, e.to_string()))),
        };
        (JsonStore { path, current: Mutex::new(value) }, unreadable)
    }

    /// A copy of the value as it is now.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.current.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Change the value and write the file. The change is kept in memory even when the write fails; the error says why
    /// the file wasn't written.
    pub fn update(&self, change: impl FnOnce(&mut T)) -> io::Result<()> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut current);
        let json = serde_json::to_vec_pretty(&*current).map_err(io::Error::other)?;
        write_atomically(&self.path, &json)
    }
}

/// Move the unreadable file aside, so a crash or a save never leaves it half-overwritten: to `<name>.bad`, or the next
/// free `<name>.bad.N` when an earlier one is kept there (a rename never replaces it, `pg_platform::file::keep_aside`).
fn quarantine(path: &Path, error: String) -> Unreadable {
    let kept = pg_platform::file::keep_aside(path, ".bad").map_err(|e| e.to_string());
    Unreadable { error, kept }
}

/// Write to `<name>.tmp` beside the file, flush it to disk, then rename it over the file: a crash leaves the old file or
/// the new one, never half of either.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = beside(path, ".tmp");
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
    #[serde(default)]
    struct Sample {
        name: String,
        count: u32,
    }

    /// A fresh folder for one test (the process id keeps parallel runs apart); removed at the start of each run.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pg-store-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = scratch("round_trip");
        let path = dir.join("sample.json");
        let (store, unreadable) = JsonStore::<Sample>::open(path.clone());
        assert!(unreadable.is_none());
        store
            .update(|s| {
                s.name = "kept".into();
                s.count = 7;
            })
            .unwrap();
        assert!(!beside(&path, ".tmp").exists(), "the temp file is renamed away");

        let (reopened, unreadable) = JsonStore::<Sample>::open(path);
        assert!(unreadable.is_none());
        assert_eq!(reopened.get(), Sample { name: "kept".into(), count: 7 });
    }

    #[test]
    fn a_missing_file_gives_the_default() {
        let dir = scratch("missing");
        let (store, unreadable) = JsonStore::<Sample>::open(dir.join("absent.json"));
        assert!(unreadable.is_none());
        assert_eq!(store.get(), Sample::default());
    }

    #[test]
    fn a_corrupt_file_is_quarantined_and_the_default_used() {
        let dir = scratch("corrupt");
        let path = dir.join("sample.json");
        std::fs::write(&path, "{ not json").unwrap();

        let (store, unreadable) = JsonStore::<Sample>::open(path.clone());
        let unreadable = unreadable.expect("the file didn't parse");
        let bad = dir.join("sample.json.bad");
        assert_eq!(unreadable.kept.as_ref().ok(), Some(&bad));
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ not json", "the content is kept");
        assert!(!path.exists(), "the corrupt file was moved, not copied");
        assert_eq!(store.get(), Sample::default());
    }

    /// A second unreadable file is kept beside the first, which is not overwritten: each failure's content stays.
    #[test]
    fn a_second_unreadable_file_keeps_the_first_one_kept() {
        let dir = scratch("corrupt_twice");
        let path = dir.join("sample.json");
        for (content, kept) in [("{ first", "sample.json.bad"), ("{ second", "sample.json.bad.1")] {
            std::fs::write(&path, content).unwrap();
            let (_, unreadable) = JsonStore::<Sample>::open(path.clone());
            assert_eq!(unreadable.unwrap().kept.ok(), Some(dir.join(kept)));
        }
        assert_eq!(std::fs::read_to_string(dir.join("sample.json.bad")).unwrap(), "{ first", "the first is kept");
        assert_eq!(std::fs::read_to_string(dir.join("sample.json.bad.1")).unwrap(), "{ second");
    }

    #[test]
    fn a_partial_file_loads_with_defaults_for_the_missing_fields() {
        let dir = scratch("partial");
        let path = dir.join("sample.json");
        std::fs::write(&path, r#"{"count": 3}"#).unwrap();
        let (store, unreadable) = JsonStore::<Sample>::open(path);
        assert!(unreadable.is_none());
        assert_eq!(store.get(), Sample { name: String::new(), count: 3 });
    }
}
