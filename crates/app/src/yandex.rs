//! Reverse image search on Yandex (`pg_yandex`): the other sizes of a photo and the images that look like it, and the
//! saving of the results the user picks. The search and the saves run on background threads. The front end subscribes
//! ([`Search::subscribe`]) and calls [`Search::poll`] on each change, and shows what comes back ([`Poll::busy`] says
//! whether one is still running). Results and words are typed values: nothing here
//! formats text or prints. The files go into the folder the user picks, named after the photo (`IMG_1234 (yandex
//! 1920x1080).jpg`), and that folder is remembered in the preferences (`yandex_folder`) for the next save.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use pg_jobs::events::{Bus, Subscription};
use pg_model::format::Format;
use pg_model::rules::PathRules;
use pg_platform::file::TempFile;
use pg_platform::names::{path_buf, rule_path_of};
pub use pg_yandex::YandexError;
use pg_yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, Fetcher, OtherSize, Similar, Upload};

use crate::prefs::PrefStore;

pub mod images;
pub use images::Fetch;
use images::Images;
pub mod source;
pub use source::Source;
pub mod replay;
pub use replay::{Recording, Replay};

impl Source for Client {
    fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError> {
        Client::upload(self, image)
    }

    fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, YandexError> {
        Client::other_sizes(self, up)
    }

    fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, YandexError> {
        Client::similar(self, up, max_pages)
    }
}

/// What a search gets from its front end: the source of its calls (Yandex's search, which runs on the search's thread),
/// and the fetcher of the results' files (shared by the threads that fetch them).
pub struct Connection {
    pub source: Box<dyn Source + Send>,
    pub fetch: Arc<dyn Fetch>,
}

/// Makes the connection for one search. The front end passes [`client()`], or [`replay()`] when it is given a recorded
/// session explicitly.
pub type Factory = Arc<dyn Fn() -> Result<Connection, YandexError> + Send + Sync>;

/// The real Yandex client, made for each search. The default.
pub fn client() -> Factory {
    Arc::new(|| -> Result<Connection, YandexError> {
        Ok(Connection { source: Box::new(Client::new()?), fetch: Arc::new(Fetcher::new()) })
    })
}

/// The recorded session in `dir` (the folder's layout is in the `replay` module), standing in for the client. It is read once, here; when it
/// can't be read, every search fails with the reason, so nothing is sent to Yandex in its place.
pub fn replay(dir: &Path) -> Factory {
    let recording = Recording::read(dir).map(Arc::new);
    Arc::new(move || match &recording {
        Ok(recording) => Ok(Connection {
            source: Box::new(Replay::new(recording.clone())),
            fetch: Arc::new(Replay::new(recording.clone())),
        }),
        Err(why) => Err(YandexError::Recorded(why.clone())),
    })
}

/// The grid picture of the result `id` of the search on screen: fetched now if it is not kept yet.
pub fn image(id: u64) -> Result<Arc<images::Fetched>, YandexError> {
    images::current().ok_or(YandexError::Unexpected("no search"))?.get_known(id)
}

/// One result as the panel shows and saves it.
#[derive(Clone, Debug)]
pub struct Hit {
    /// The result's id: its search and its place in the search's results (`hit_id`). Its picture is kept under it.
    pub id: u64,
    /// The file the result is, at its host.
    pub url: String,
    pub width: i32,
    pub height: i32,
    /// What follows the photo's name in the saved file's name, inside the brackets: `yandex 1920x1080`.
    pub suffix: String,
}

/// What a finished search found: the other sizes, then the similar images.
#[derive(Debug)]
pub struct Found {
    pub other: Vec<Hit>,
    pub similar: Vec<Hit>,
}

/// The id of the result at `index` of the search numbered `generation` (numbered from 1, so no id is 0).
fn hit_id(generation: u64, index: usize) -> u64 {
    // A search has far fewer results than 2^32, so the index never reaches the generation's bits.
    (generation << 32) | u64::try_from(index).unwrap_or(u64::MAX)
}

/// Where a search or a save is, while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Uploading,
    OtherSizes,
    Similar,
    /// A save is about to write `files` files.
    Saving {
        files: usize,
    },
    /// The `n`th of `of` files (counted from 1) is being downloaded.
    SavingFile {
        n: usize,
        of: usize,
    },
}

/// A search that ended with a note: the similar images stopped early or failed. The other sizes are still found.
#[derive(Debug)]
pub enum SimilarNote {
    /// The list stopped after its first page: `shown` items were read before `error`.
    Stopped { shown: usize, error: YandexError },
    /// The similar images could not be read.
    Failed(YandexError),
}

/// Why a search ended without results.
#[derive(Debug)]
pub enum SearchError {
    /// Refused before anything is sent: only still images are searched. Decided by the file's first bytes, so a
    /// video is refused without being read.
    NotStill,
    /// The photo could not be read from disk.
    Unreadable(std::io::Error),
    /// The photo is in a format Yandex may refuse (TIFF), and it could not be decoded to be sent as a JPEG.
    Undecodable(String),
    /// A request to Yandex failed before any results came (the client, the upload or the other sizes).
    Request(YandexError),
}

/// Why one save failed. The first one is kept in [`Saved`].
#[derive(Debug)]
pub enum SaveFailure {
    /// The client could not be made, so nothing was downloaded.
    Client(YandexError),
    /// A file could not be downloaded.
    Download { url: String, error: YandexError },
    /// A downloaded file could not be written.
    Write { url: String, path: PathBuf, error: std::io::Error },
}

/// What a save did: where, how many files were written and how many failed, and the first failure.
#[derive(Debug)]
pub struct Saved {
    pub folder: PathBuf,
    pub written: usize,
    pub failed: usize,
    pub first_error: Option<SaveFailure>,
}

/// A step's outcome, as the front end reads it from [`Poll::status`].
#[derive(Debug)]
pub enum Status {
    /// A step in progress.
    Working(Phase),
    /// The search ended. The note is there when the similar images stopped early or failed.
    Searched(Option<SimilarNote>),
    /// The search ended without results.
    SearchFailed(SearchError),
    /// The save ended.
    Saved(Saved),
    /// The search or the save stopped at an internal error (the panic is in the log).
    Stopped,
}

/// What the front end reads on each poll.
#[derive(Debug)]
pub struct Poll {
    /// A search or a save is running.
    pub busy: bool,
    /// The status since the last poll, when it changed.
    pub status: Option<Status>,
    /// The results that came in since the last poll, once.
    pub found: Option<Found>,
    /// The results whose files could not be fetched or decoded since the last poll, once: take them off the grid.
    pub dead: Vec<u64>,
}

/// What the background threads share with [`Search::poll`].
#[derive(Default)]
struct Shared {
    busy: bool,
    /// How many searches have started: the results' ids say which search they are from ([`hit_id`]).
    generation: u64,
    /// The photo searched for: the saved files are named after it.
    photo: PathBuf,
    /// The latest status, not yet read by `poll`.
    status: Option<Status>,
    /// The results, not yet read by `poll`.
    found: Option<Found>,
}

/// One reverse image search at a time, and the saves of its results. A search or a save that starts while another runs
/// is refused. Every change to what `poll` reports is also announced on `changes`, so a front end need not poll.
pub struct Search {
    shared: Arc<Mutex<Shared>>,
    changes: Arc<Bus<()>>,
}

impl Default for Search {
    fn default() -> Self {
        // Each subscriber keeps a few changes; a slow one loses the oldest, and `poll` reads the whole state anyway.
        Search { shared: Arc::default(), changes: Arc::new(Bus::new(16)) }
    }
}

/// How much of a file is read to tell its format ([`Format::sniff`] looks at no more).
const SNIFF_BYTES: u64 = 16;

/// What the search uploads of the photo at `path`: its bytes, read once from one open of the file. See
/// [`photo_bytes`].
fn read_photo(path: &Path) -> Result<Vec<u8>, SearchError> {
    let file = File::open(path).map_err(SearchError::Unreadable)?;
    photo_bytes(file)
}

/// The bytes to upload of the photo `file` holds, decided by its content, not its name. Its first bytes are read and
/// sniffed, and anything but a still image (a video, an unknown file) is refused before the rest is read. JPEG, PNG,
/// WebP, GIF and BMP are sent as they are: the set the search has always taken. A TIFF, which Yandex may refuse, is
/// sent as a JPEG of the same size ([`pg_render::transcode::to_jpeg`]).
fn photo_bytes(mut file: impl Read) -> Result<Vec<u8>, SearchError> {
    let mut bytes = Vec::new();
    file.by_ref().take(SNIFF_BYTES).read_to_end(&mut bytes).map_err(SearchError::Unreadable)?;
    let format = Format::sniff(&bytes).ok_or(SearchError::NotStill)?;
    file.read_to_end(&mut bytes).map_err(SearchError::Unreadable)?;
    match format {
        Format::Jpeg | Format::Png | Format::Webp | Format::Gif | Format::Bmp => Ok(bytes),
        Format::Tiff => pg_render::transcode::to_jpeg(&bytes).map_err(|e| SearchError::Undecodable(format!("{e:#}"))),
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A thread that panicked must not stop the next search, so a poisoned lock is still used.
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_status(shared: &Mutex<Shared>, changes: &Bus<()>, status: Status) {
    lock(shared).status = Some(status);
    changes.publish(());
}

fn dimension(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The search, on its own thread: upload, other sizes, similar. Returns what it found, with a note when the similar
/// images stopped or failed (the other sizes are still shown, as the crawler keeps them).
fn find_similar<S: Source + ?Sized>(
    photo: &Path,
    source: &mut S,
    shared: &Mutex<Shared>,
    changes: &Bus<()>,
) -> Result<(Found, Option<SimilarNote>), SearchError> {
    let bytes = read_photo(photo)?;
    set_status(shared, changes, Status::Working(Phase::Uploading));
    let upload = source.upload(&bytes).map_err(SearchError::Request)?;
    set_status(shared, changes, Status::Working(Phase::OtherSizes));
    let other = source.other_sizes(&upload).map_err(SearchError::Request)?;
    set_status(shared, changes, Status::Working(Phase::Similar));
    let (similar, note) = match source.similar(&upload, DEFAULT_MAX_SIMILAR_PAGES) {
        Ok(list) => {
            let shown = list.items.len();
            let note = list.stopped.map(|error| SimilarNote::Stopped { shown, error });
            (list.items, note)
        }
        Err(error) => (Vec::new(), Some(SimilarNote::Failed(error))),
    };
    // The ids are numbered when the search's results are known (`number`).
    let found = Found {
        other: other
            .into_iter()
            .map(|s| Hit {
                id: 0,
                url: s.url,
                width: dimension(s.width),
                height: dimension(s.height),
                suffix: format!("yandex {}x{}", s.width, s.height),
            })
            .collect(),
        similar: similar
            .into_iter()
            .map(|s| Hit {
                id: 0,
                url: s.image_url,
                width: dimension(s.width),
                height: dimension(s.height),
                suffix: format!("yandex similar {}", s.pos + 1),
            })
            .collect(),
    };
    Ok((found, note))
}

/// Gives the results of a search their ids: the search `generation` and each result's place, the other sizes first.
fn number(found: &mut Found, generation: u64) {
    let hits = found.other.iter_mut().chain(found.similar.iter_mut());
    for (index, hit) in hits.enumerate() {
        hit.id = hit_id(generation, index);
    }
}

/// The photo's stem as a saved file's: `stem` kept exactly, then ` (<suffix>)`. The suffix is text the search returns, so
/// the characters Windows refuses in names become `_`.
fn saved_stem(stem: &OsStr, suffix: &str) -> OsString {
    let clean: String = suffix.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let mut name = stem.to_os_string();
    name.push(format!(" ({})", clean.trim()));
    name
}

/// How many names [`claim`] tries (`<stem>`, then ` (2)` up to this) before it gives up.
const MAX_CLAIMS: u32 = 10_000;

/// The file name `<stem>.<ext>`, or with ` (n)` after the stem when `n` is past 1.
fn numbered(stem: &OsStr, ext: &str, n: u32) -> OsString {
    let mut name = stem.to_os_string();
    if n > 1 {
        name.push(format!(" ({n})"));
    }
    name.push(format!(".{ext}"));
    name
}

/// Takes the first free name of `folder/<stem>.<ext>`, ` (2)`, ` (3)`… by making it, empty: `create_new` makes the
/// check and the claim one step, so a name another program takes at the same time is not written over.
fn claim(folder: &Path, stem: &OsStr, ext: &str) -> io::Result<PathBuf> {
    for n in 1..=MAX_CLAIMS {
        let path = folder.join(numbered(stem, ext, n));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(ErrorKind::AlreadyExists, format!("{MAX_CLAIMS} names of this file are all taken")))
}

/// The temporary a save writes before it takes the final name: `.<stem>.<ext>.pg-save` beside it (the repository's
/// temporaries are dot-names beside their target: `.pg-write`, `.pg-crop`).
fn temp_path(folder: &Path, stem: &OsStr, ext: &str) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(numbered(stem, ext, 1));
    name.push(".pg-save");
    folder.join(name)
}

/// Writes `bytes` to a new file in `folder` named `<stem>.<ext>` (or ` (2)`, ` (3)`… after the stem when that is
/// taken) and returns its path. The bytes go to a temporary beside it first ([`temp_path`]), synced; then the final
/// name is claimed ([`claim`]) and the temporary renamed over the empty claim. A failure at any step removes both, so
/// a failed write leaves no partial file at the photo's name. The error comes with the path it was at.
///
/// What a crash leaves: during the write, the temporary (its name is fixed, so the next save of that name replaces
/// it); in the instant between the claim and the rename, the temporary and an empty file at the claimed name.
fn write_new(folder: &Path, stem: &OsStr, ext: &str, bytes: &[u8]) -> Result<PathBuf, (PathBuf, io::Error)> {
    write_new_with(folder, stem, ext, bytes, |from, to| std::fs::rename(from, to))
}

/// [`write_new`], with the rename that puts the temporary at the claimed name given as `rename` (the seam a test
/// uses to make that step fail, after the claim).
fn write_new_with(
    folder: &Path,
    stem: &OsStr,
    ext: &str,
    bytes: &[u8],
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, (PathBuf, io::Error)> {
    let temp = TempFile::new(&temp_path(folder, stem, ext));
    let at_temp = |error: io::Error| (temp.path().to_path_buf(), error);
    let mut file = File::create(temp.path()).map_err(at_temp)?;
    file.write_all(bytes).map_err(at_temp)?;
    file.sync_all().map_err(at_temp)?;
    drop(file);
    let path = claim(folder, stem, ext).map_err(|error| (folder.join(numbered(stem, ext, 1)), error))?;
    let placeholder = TempFile::new(&path);
    rename(temp.path(), &path).map_err(|error| (path.clone(), error))?;
    placeholder.persist();
    temp.persist();
    Ok(path)
}

/// The file-name stem of the photo, kept exactly: `IMG_1234` for `IMG_1234.jpg`.
fn stem_of(photo: &Path) -> OsString {
    photo.file_stem().map_or_else(|| OsString::from("image"), OsStr::to_os_string)
}

/// Writes a result's file (kept by the search, or fetched now) into `folder`, named `<stem> (<suffix>)` so it sorts
/// next to the photo ([`write_new`]).
fn save_one(images: &Images, hit: &Hit, stem: &OsStr, folder: &Path) -> Result<(), SaveFailure> {
    let kept = images.get(hit.id, &hit.url).map_err(|error| SaveFailure::Download { url: hit.url.clone(), error })?;
    write_new(folder, &saved_stem(stem, &hit.suffix), kept.file.format.extension(), &kept.file.bytes)
        .map(drop)
        .map_err(|(path, error)| SaveFailure::Write { url: hit.url.clone(), path, error })
}

/// What a search found, its note, and the fetcher of its results' files.
type Searched = (Found, Option<SimilarNote>, Arc<dyn Fetch>);

/// The search on its thread: its connection is made first (a recorded session or the client), then the search runs on
/// its source. The fetcher goes with the results, for their files.
fn search(photo: &Path, make: &Factory, shared: &Mutex<Shared>, changes: &Bus<()>) -> Result<Searched, SearchError> {
    let mut connection = make().map_err(SearchError::Request)?;
    let (found, note) = find_similar(photo, &mut *connection.source, shared, changes)?;
    Ok((found, note, connection.fetch))
}

/// The outcome of a save with no search on screen to take its files from: nothing downloaded, every hit failed.
fn save_refused(hits: &[Hit], folder: &Path, error: YandexError) -> Saved {
    let failed = hits.len();
    Saved { folder: folder.to_path_buf(), written: 0, failed, first_error: Some(SaveFailure::Client(error)) }
}

/// Writes every hit into `folder`, one at a time, from the search's images.
fn save_all(
    hits: &[Hit],
    stem: &OsStr,
    folder: &Path,
    images: &Images,
    shared: &Mutex<Shared>,
    changes: &Bus<()>,
) -> Saved {
    let mut saved = Saved { folder: folder.to_path_buf(), written: 0, failed: 0, first_error: None };
    for (i, hit) in hits.iter().enumerate() {
        set_status(shared, changes, Status::Working(Phase::SavingFile { n: i + 1, of: hits.len() }));
        match save_one(images, hit, stem, folder) {
            Ok(()) => saved.written += 1,
            Err(failure) => {
                saved.failed += 1;
                saved.first_error.get_or_insert(failure);
            }
        }
    }
    saved
}

impl Search {
    /// Runs `work` (a search or a save, which has set `busy`) on a thread of its own, named `name`. When the system
    /// refuses the thread, `busy` and the status are cleared again, the change is announced, and the error returned:
    /// nothing ran, so the next search or save may start.
    fn spawn_worker(&self, name: &str, work: impl FnOnce() + Send + 'static) -> io::Result<bool> {
        let name = name.to_string();
        self.spawn_with(work, |work| thread::Builder::new().name(name).spawn(work).map(drop))
    }

    /// [`Search::spawn_worker`] with the spawn given as `spawn`, which the tests make fail.
    fn spawn_with<F: FnOnce() + Send + 'static>(
        &self,
        work: F,
        spawn: impl FnOnce(F) -> io::Result<()>,
    ) -> io::Result<bool> {
        match spawn(work) {
            Ok(()) => Ok(true),
            Err(e) => {
                let mut s = lock(&self.shared);
                s.busy = false;
                s.status = None;
                drop(s);
                self.changes.publish(());
                Err(e)
            }
        }
    }

    /// Start a search for the photo at `photo`, a local path, with the source `make` gives (the client, or a recorded
    /// session). False when a search or a save is running (nothing changes); an error when its thread couldn't start
    /// (nothing runs, and nothing stays busy).
    pub fn start(&self, photo: &Path, make: Factory) -> io::Result<bool> {
        let photo = photo.to_path_buf();
        {
            let mut s = lock(&self.shared);
            if s.busy {
                return Ok(false);
            }
            s.busy = true;
            s.photo = photo.clone();
            s.status = Some(Status::Working(Phase::Starting));
        }
        self.changes.publish(());
        let shared = self.shared.clone();
        let changes = self.changes.clone();
        self.spawn_worker("yandex search", move || {
            // A panic here is logged (the hook), and the search ends with a status, so `busy` is always cleared.
            let outcome = pg_jobs::panics::catch(AssertUnwindSafe(|| search(&photo, &make, &shared, &changes)));
            let mut s = lock(&shared);
            // The results' files are fetched after the search ends, into this search's images (which replace the last
            // search's on screen).
            let mut fetching = None;
            match outcome {
                Ok(Ok((mut found, note, fetch))) => {
                    s.generation += 1;
                    number(&mut found, s.generation);
                    let results: Vec<(u64, String)> =
                        found.other.iter().chain(&found.similar).map(|hit| (hit.id, hit.url.clone())).collect();
                    let pictures = Images::new(fetch, images::grid_picture, changes.clone());
                    images::set_current(Some(pictures.clone()));
                    fetching = Some((pictures, results));
                    s.found = Some(found);
                    s.status = Some(Status::Searched(note));
                }
                Ok(Err(e)) => s.status = Some(Status::SearchFailed(e)),
                Err(_) => s.status = Some(Status::Stopped),
            }
            // Same lock as the results, so `poll()` never sees the search end before its results.
            s.busy = false;
            drop(s);
            if let Some((pictures, results)) = fetching {
                pictures.prefetch(results);
            }
            changes.publish(());
        })
    }

    /// A subscriber's end of the change bus: an event each time what `poll` reports changes. Events are only a signal;
    /// the state is read with `poll`. Subscribe before `start` or `save`, or the first changes are missed.
    pub fn subscribe(&self) -> Subscription<()> {
        self.changes.subscribe()
    }

    /// The background state, read by the front end while `busy`: the results that came in, and the status since the last
    /// poll. Each result and status is returned once.
    pub fn poll(&self) -> Poll {
        let mut s = lock(&self.shared);
        let dead = images::current().map(|pictures| pictures.take_dead()).unwrap_or_default();
        Poll { busy: s.busy, status: s.status.take(), found: s.found.take(), dead }
    }

    /// Where the save dialog opens: the folder saved into last (`yandex_folder`) while it exists, or else the photo's
    /// folder. None when there is neither.
    pub fn start_folder(&self, prefs: &PrefStore) -> Option<PathBuf> {
        let last = prefs.get().yandex_folder.map(|folder| path_buf(&PathRules::NATIVE.join(&folder)));
        if let Some(last) = last.filter(|last| last.is_dir()) {
            return Some(last);
        }
        let photo = lock(&self.shared).photo.clone();
        photo.parent().map(Path::to_path_buf)
    }

    /// Save the picked `hits` into `folder`, one file each, named after the photo, from the files the search kept. The
    /// folder is remembered in the preferences for the next save, whether or not every save works. False when nothing
    /// is picked or a search or a save is running (nothing changes); an error when its thread couldn't start (nothing
    /// is written, and nothing stays busy). The save's outcome comes from [`Search::poll`].
    pub fn save(&self, prefs: &PrefStore, hits: Vec<Hit>, folder: PathBuf) -> io::Result<bool> {
        if hits.is_empty() {
            return Ok(false);
        }
        let stem = {
            let mut s = lock(&self.shared);
            if s.busy {
                return Ok(false);
            }
            s.busy = true;
            s.status = Some(Status::Working(Phase::Saving { files: hits.len() }));
            stem_of(&s.photo)
        };
        self.changes.publish(());
        // A preferences file that can't be written keeps the folder in memory for this run; the save goes on. The app
        // reports that failure through its notes when it can be reported.
        if let Some(chosen) = rule_path_of(&folder) {
            let _ = prefs.update(|p| p.yandex_folder = Some(chosen));
        }
        let shared = self.shared.clone();
        let changes = self.changes.clone();
        self.spawn_worker("yandex save", move || {
            // A panic here is logged (the hook), and the save ends with a status, so `busy` is always cleared.
            let status = match pg_jobs::panics::catch(AssertUnwindSafe(|| match images::current() {
                Some(pictures) => save_all(&hits, &stem, &folder, &pictures, &shared, &changes),
                None => save_refused(&hits, &folder, YandexError::Unexpected("no search on screen")),
            })) {
                Ok(saved) => Status::Saved(saved),
                Err(_) => Status::Stopped,
            };
            let mut s = lock(&shared);
            s.status = Some(status);
            s.busy = false;
            drop(s);
            changes.publish(());
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stem that is no UTF-8: a lone surrogate on Windows, a raw `0xFF` byte on Unix.
    fn odd_stem() -> OsString {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[u16::from(b'I'), 0xD800])
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"IMG_\xFF".to_vec())
        }
    }

    /// The names in `folder`, sorted.
    fn names_in(folder: &Path) -> Vec<OsString> {
        let mut names: Vec<OsString> =
            std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        names.sort();
        names
    }

    #[test]
    fn saved_names_follow_the_photo() {
        let folder = pg_testkit::test_folder("scratch_app", "yandex-names").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        let path = write_new(&folder, &stem, "jpg", b"first").unwrap();
        assert_eq!(path, folder.join("IMG_1234 (yandex 1920x1080).jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(names_in(&folder), [OsString::from("IMG_1234 (yandex 1920x1080).jpg")], "no temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn two_saves_of_one_name_get_two_files() {
        let folder = pg_testkit::test_folder("scratch_app", "yandex-twice").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        let first = write_new(&folder, &stem, "jpg", b"first").unwrap();
        let second = write_new(&folder, &stem, "jpg", b"second").unwrap();
        assert_eq!(second, folder.join("IMG_1234 (yandex 1920x1080) (2).jpg"));
        assert_eq!(std::fs::read(&first).unwrap(), b"first", "the first is not written over");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        assert_eq!(names_in(&folder).len(), 2, "no temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_file_at_the_name() {
        let folder = pg_testkit::test_folder("scratch_app", "yandex-failed").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        // A folder where the temporary goes: the write to it fails.
        let temp = temp_path(&folder, &stem, "jpg");
        std::fs::create_dir(&temp).unwrap();
        let (at, _) = write_new(&folder, &stem, "jpg", b"bytes").unwrap_err();
        assert_eq!(at, temp, "the error names where it failed");
        assert!(!folder.join("IMG_1234 (yandex 1920x1080).jpg").exists(), "no partial or empty file at the name");
        assert_eq!(names_in(&folder), [temp.file_name().unwrap().to_os_string()], "only the folder put there");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_rename_after_the_claim_leaves_no_file_at_the_name() {
        let folder = pg_testkit::test_folder("scratch_app", "yandex-renamed").unwrap();
        let stem = saved_stem(OsStr::new("IMG_1234"), "yandex 1920x1080");
        // The claim (an empty file at the name) is made, then the rename of the temporary onto it is refused.
        let refused = write_new_with(&folder, &stem, "jpg", b"bytes", |_, _| Err(io::Error::other("rename refused")));
        let (at, error) = refused.unwrap_err();
        assert_eq!(at, folder.join("IMG_1234 (yandex 1920x1080).jpg"), "the error names the claimed name");
        assert_eq!(error.to_string(), "rename refused");
        assert!(names_in(&folder).is_empty(), "neither the claim nor the temporary is left");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_thread_that_cannot_start_clears_busy_and_the_status() {
        let search = Search::default();
        {
            let mut s = lock(&search.shared);
            s.busy = true;
            s.status = Some(Status::Working(Phase::Starting));
        }
        let refused = search.spawn_with(|| {}, |_work| Err(io::Error::other("no threads left")));
        assert!(matches!(refused, Err(ref e) if e.to_string() == "no threads left"));
        let s = lock(&search.shared);
        assert!(!s.busy, "the next search or save may start");
        assert!(s.status.is_none(), "no status is left for a search that never ran");
    }

    #[test]
    fn a_thread_that_starts_runs_its_work() {
        let search = Search::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let started = search.spawn_with(
            move || {
                let _ = tx.send(());
            },
            |work| thread::Builder::new().spawn(work).map(drop),
        );
        assert!(matches!(started, Ok(true)));
        assert!(rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the work ran on its thread");
    }

    /// A reader that fails: what the photo's file gives past the bytes a test lets be read.
    struct Refuses;

    impl Read for Refuses {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read past the sniffed bytes"))
        }
    }

    #[test]
    fn a_still_image_is_sent_as_it_is() {
        let png = pg_testkit::png_rgb(2, 2, &[10; 12]).unwrap();
        assert_eq!(photo_bytes(png.as_slice()).unwrap(), png);
    }

    #[test]
    fn a_video_is_refused_on_its_first_bytes() {
        // An MP4's start (a `ftyp` box), and then a file that fails if it is read further.
        let head: &[u8] = b"\0\0\0\x18ftypmp42\0\0\0\0";
        assert_eq!(head.len(), 16);
        assert!(matches!(photo_bytes(head.chain(Refuses)), Err(SearchError::NotStill)));
        assert!(matches!(photo_bytes(&b""[..]), Err(SearchError::NotStill)), "an empty file too");
    }

    #[test]
    fn a_tiff_is_sent_as_a_jpeg_of_its_size() {
        let rgb: Vec<u8> = (0..6 * 4).flat_map(|_| [30u8, 120, 220]).collect();
        let tiff = pg_testkit::tiff_rgb(6, 4, &rgb).unwrap();
        let sent = photo_bytes(tiff.as_slice()).unwrap();
        assert_eq!(Format::sniff(&sent), Some(Format::Jpeg));
        let mut decoder = pg_codec::decode::Decoder::new().unwrap();
        let decoded = pg_render::display::decode(
            &mut decoder,
            &sent,
            pg_codec::decode::MinSize::Full,
            pg_model::turns::Turns::NONE,
        )
        .unwrap();
        assert_eq!((decoded.info.format, decoded.image.width, decoded.image.height), (Format::Jpeg, 6, 4));
    }

    #[test]
    fn a_tiff_that_doesnt_decode_is_undecodable() {
        let mut broken = b"II\x2a\x00".to_vec();
        broken.extend_from_slice(&[0xFF; 32]);
        assert!(matches!(photo_bytes(broken.as_slice()), Err(SearchError::Undecodable(_))));
    }

    #[test]
    fn a_missing_photo_is_unreadable() {
        let folder = pg_testkit::test_folder("scratch_app", "yandex-missing").unwrap();
        assert!(matches!(read_photo(&folder.join("gone.jpg")), Err(SearchError::Unreadable(_))));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_photos_stem_is_kept_exactly_and_only_the_suffix_is_cleaned() {
        let stem = odd_stem();
        let mut expected = stem.clone();
        expected.push(" (a_b_c)");
        assert_eq!(saved_stem(&stem, "a/b:c"), expected);
    }
}
