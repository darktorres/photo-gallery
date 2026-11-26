//! A JSON file the app keeps a value in: read once when it is opened, changed in memory, and written back atomically on
//! each change. Used for the preferences; the path is given at construction and resolved by the owner, not again here.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use pg_platform::file::beside;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A file that didn't read. It was moved aside to `kept` (or couldn't be, with the reason as the `Err`), and the defaults
/// are in use. The file is kept, not overwritten by the next save: its content can be copied back by hand.
#[derive(Clone, Debug)]
pub struct Unreadable {
    /// The error, as text.
    pub error: String,
    /// Where the file was kept, or why it couldn't be.
    pub kept: Result<PathBuf, String>,
}

/// The value a JSON file holds, with its path and the write order of its saves. See the module comment.
pub struct JsonStore<T> {
    path: PathBuf,
    current: Mutex<Current<T>>,
    /// The file's write order, shared by its saves (`Save::write`).
    order: Arc<Mutex<WriteOrder>>,
}

/// The value, and how many changes it has had since the file was read.
struct Current<T> {
    value: T,
    changes: u64,
}

/// The write order of a file's saves: the number of the change the file holds, and of the newest change whose save has been
/// attempted, whatever its outcome.
#[derive(Default)]
struct WriteOrder {
    written: u64,
    attempted: u64,
}

/// A change made to a `JsonStore`'s value, and the file's content it leaves. [`Save::write`] puts that content in the
/// file, on any thread: the change is in memory from the moment it is made, the file follows when the save is written.
/// A save that a later change's save has already been attempted for is not written, so an older content never replaces
/// the newer one on disk; its outcome is the later save's, so a failed later save is not reported as saved.
pub struct Save<T> {
    path: PathBuf,
    value: T,
    changes: u64,
    order: Arc<Mutex<WriteOrder>>,
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
        let current = Current { value, changes: 0 };
        let order = Arc::new(Mutex::new(WriteOrder::default()));
        (JsonStore { path, current: Mutex::new(current), order }, unreadable)
    }

    /// A copy of the value as it is now.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.current.lock().unwrap_or_else(PoisonError::into_inner).value.clone()
    }

    /// Change the value in memory, and return the save of the file. The value is not written until the save is: the
    /// lock is not held across the write, so a `get` never waits for the disk.
    pub fn change(&self, change: impl FnOnce(&mut T)) -> Save<T>
    where
        T: Clone,
    {
        let (value, changes) = {
            let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut current.value);
            current.changes += 1;
            (current.value.clone(), current.changes)
        };
        Save { path: self.path.clone(), value, changes, order: self.order.clone() }
    }

    /// Change the value and write the file. The change is kept in memory even when the write fails; the error says why
    /// the file wasn't written.
    pub fn update(&self, change: impl FnOnce(&mut T)) -> io::Result<()>
    where
        T: Clone,
    {
        self.change(change).write()
    }
}

impl<T: Serialize> Save<T> {
    /// Write the file with this change, unless a later change's save has been attempted already. The file is replaced
    /// atomically (`write_atomically`). An overtaken save is not written: it is `Ok` when the later save went through,
    /// and an error when it did not, since the file then holds neither change.
    pub fn write(self) -> io::Result<()> {
        let mut order = self.order.lock().unwrap_or_else(PoisonError::into_inner);
        if self.changes < order.attempted {
            return if order.written >= order.attempted {
                Ok(())
            } else {
                Err(io::Error::other("a later change to the file was not saved, so this one was not written either"))
            };
        }
        order.attempted = self.changes;
        let json = serde_json::to_vec_pretty(&self.value).map_err(io::Error::other)?;
        write_atomically(&self.path, &json)?;
        order.written = self.changes;
        Ok(())
    }
}

/// Move the unreadable file aside, so a crash or a save never leaves it half-overwritten: to `<name>.bad`, or the next
/// free `<name>.bad.N` when an earlier one is kept there (a rename never replaces it, `pg_platform::file::move_aside`).
fn quarantine(path: &Path, error: String) -> Unreadable {
    let kept = pg_platform::file::move_aside(path, &[""], ".bad").map_err(|e| e.to_string());
    Unreadable { error, kept }
}

/// Write to `<name>.tmp` beside the file, flush it to disk, then rename it over the file: a crash leaves the old file or
/// the new one, never half of either.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = beside(path, ".tmp");
    pg_platform::file::write_synced(&temp, bytes)?;
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

    /// A fresh folder for one test, a run folder of its own (`pg_testkit::run_folder`).
    fn scratch(test: &str) -> PathBuf {
        pg_testkit::run_folder("scratch_app", &format!("store-{test}")).unwrap()
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = scratch("round-trip");
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

    /// Saves written out of order leave the file with the latest change: an earlier save written after a later one is
    /// not written, and the value in memory is the latest change either way.
    #[test]
    fn a_save_overtaken_by_a_later_one_is_not_written() {
        let dir = scratch("overtaken");
        let path = dir.join("sample.json");
        let (store, _) = JsonStore::<Sample>::open(path.clone());
        let first = store.change(|s| s.count = 1);
        let second = store.change(|s| s.count = 2);
        second.write().unwrap();
        first.write().unwrap();
        assert_eq!(store.get().count, 2);
        let (reopened, _) = JsonStore::<Sample>::open(path);
        assert_eq!(reopened.get().count, 2, "the file holds the later change");
    }

    /// A later save that failed is not followed by an earlier save's content: the earlier save is not written, and says so,
    /// so the caller keeps its failure instead of reading the older save as the file being saved.
    #[test]
    fn an_earlier_save_after_a_failed_later_one_is_not_written() {
        let dir = scratch("failed-later");
        let path = dir.join("sample.json");
        let (store, _) = JsonStore::<Sample>::open(path.clone());
        let first = store.change(|s| s.count = 1);
        let second = store.change(|s| s.count = 2);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(second.write().is_err(), "the later save cannot write into a missing folder");
        assert!(first.write().is_err(), "the earlier save is overtaken by the failed later one");
        assert!(!path.exists() && !dir.exists(), "neither change reached the file");
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
        let dir = scratch("corrupt-twice");
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
