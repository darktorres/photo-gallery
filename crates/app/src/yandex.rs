//! Reverse image search on Yandex (`pg_yandex`): the other sizes of a photo and the images that look like it, and the
//! saving of the results the user picks: the app's [`Yandex`] service. The search and the saves run on background
//! threads. The front end subscribes ([`Yandex::subscribe`]) and calls [`Yandex::poll`] on each change, and shows what
//! comes back ([`Poll::busy`] says whether one is still running). Results and outcomes are typed values
//! (`pg_model::yandex`): nothing here formats text or prints. The files go into the folder the user picks, named after the photo (`IMG_1234 (yandex
//! 1920x1080).jpg`), and that folder is remembered in the preferences (`yandex_folder`) for the next save.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use pg_jobs::events::{Bus, Subscription};
use pg_model::format::Format;
use pg_model::yandex::{Phase, SaveFailure, Saved, SearchError, SimilarNote, Status};
use pg_platform::names::{disk_path, rule_path_of, shown};
use pg_platform::native::PATH_RULES;
use pg_yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, Fetcher, OtherSize, Similar, Upload, YandexError};

use crate::makers::Makers;
use crate::prefs::PrefStore;

pub mod images;
pub use images::Fetch;
use images::{Check, Fetched, Images};
pub mod source;
pub use source::{Source, SourceError};
pub mod replay;
pub use replay::{Recording, Replay};
mod save;
use save::{saved_stem, stem_of, write_new};

impl Source for Client {
    fn upload(&mut self, image: &[u8]) -> Result<Upload, SourceError> {
        Ok(Client::upload(self, image)?)
    }

    fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, SourceError> {
        Ok(Client::other_sizes(self, up)?)
    }

    fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, SourceError> {
        Ok(Client::similar(self, up, max_pages)?)
    }
}

/// What a search gets from its front end: the source of its calls (Yandex's search, which runs on the search's thread),
/// and the fetcher of the results' files (shared by the threads that fetch them).
pub struct Connection {
    pub source: Box<dyn Source + Send>,
    pub fetch: Arc<dyn Fetch>,
}

/// Where the searches get their calls: Yandex (the default), or the recorded session in a folder (the `replay`
/// module's layout), which answers in its place and sends nothing. Only an explicit choice selects a recording, because
/// a search sends the photo to Yandex.
#[derive(Clone, Debug, Default)]
pub enum YandexSource {
    #[default]
    Live,
    Replay(PathBuf),
}

/// Makes the connection for one search: [`client()`], or [`replay()`] for a recorded session.
pub type Factory = Arc<dyn Fn() -> Result<Connection, SourceError> + Send + Sync>;

/// The real Yandex client, made for each search. The default.
pub fn client() -> Factory {
    Arc::new(|| -> Result<Connection, SourceError> {
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
        Err(why) => Err(SourceError::Recorded(why.clone())),
    })
}

/// The app's reverse image search: one [`Search`] at a time, its calls made by the configured source, its grid
/// pictures made by the app's makers, and the save folder remembered in the preferences.
pub struct Yandex {
    search: Search,
    factory: Factory,
    prefs: Arc<PrefStore>,
    makers: Arc<Makers>,
}

/// How long [`Yandex::stop`] waits for a search, a save and the fetchers to end. A request to Yandex can't be cut short,
/// so a slow one is left to end with the process.
const STOP_WAIT: Duration = Duration::from_secs(2);

impl Yandex {
    pub(crate) fn new(source: &YandexSource, prefs: Arc<PrefStore>, makers: Arc<Makers>) -> Yandex {
        let factory = match source {
            YandexSource::Live => client(),
            YandexSource::Replay(dir) => {
                pg_log::info(
                    "yandex",
                    &format!("answering from the recorded session in {}; nothing is sent", dir.display()),
                );
                replay(dir)
            }
        };
        Yandex { search: Search::default(), factory, prefs, makers }
    }

    /// Start a search for the photo at `photo`, a local path, its results' grid pictures `grid_height` px high (the
    /// panel's rows). False when a search or a save is running, or the service has stopped (nothing changes); an error
    /// when its thread couldn't start (nothing runs, and nothing stays busy).
    pub fn start(&self, photo: &Path, grid_height: usize) -> io::Result<bool> {
        let makers = self.makers.clone();
        let check: Check = Arc::new(move |bytes| images::grid_picture(&makers, bytes, grid_height));
        self.search.start(photo, self.factory.clone(), check)
    }

    /// See [`Search::subscribe`].
    pub fn subscribe(&self) -> Subscription<()> {
        self.search.subscribe()
    }

    /// See [`Search::poll`].
    pub fn poll(&self) -> Poll {
        self.search.poll()
    }

    /// See [`Search::start_folder`].
    pub fn start_folder(&self) -> Option<PathBuf> {
        self.search.start_folder(&self.prefs)
    }

    /// See [`Search::save`].
    pub fn save(&self, hits: Vec<Hit>, folder: PathBuf) -> io::Result<bool> {
        self.search.save(&self.prefs, hits, folder)
    }

    /// The grid picture of the result `id` of the search on screen: fetched now if it is not kept yet.
    pub fn image(&self, id: u64) -> Result<Arc<Fetched>, SourceError> {
        self.search.image(id)
    }

    /// No search or save starts from now on, and the fetchers take no more results; waits a while for what runs to end.
    /// False when something still ran after the wait.
    pub(crate) fn stop(&self) -> bool {
        self.search.stop(STOP_WAIT)
    }
}

/// One result as the panel shows and saves it.
#[derive(Clone, Debug)]
pub struct Hit {
    /// The result's id: its search and its place in the search's results (`hit_id`). Its picture is kept under it.
    pub id: u64,
    /// The file the result is, at its host.
    pub url: String,
    /// Its size as Yandex gives it, in pixels; 0 when it gives none.
    pub width: u32,
    pub height: u32,
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

/// What the front end reads on each poll.
#[derive(Debug)]
pub struct Poll {
    /// A search or a save is running.
    pub busy: bool,
    /// The status since the last poll, when it changed: data, which the front end words.
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
    /// Set by [`Search::stop`]: nothing starts any more.
    closed: bool,
}

/// One reverse image search at a time, and the saves of its results. A search or a save that starts while another runs
/// is refused. Every change to what `poll` reports is also announced on `changes`, so a front end need not poll.
pub struct Search {
    shared: Arc<Mutex<Shared>>,
    changes: Arc<Bus<()>>,
    /// The results' files of the search on screen: what the grid's picture requests and a save read. A new search
    /// replaces them.
    pictures: Arc<RwLock<Option<Arc<Images>>>>,
}

impl Default for Search {
    fn default() -> Self {
        // Each subscriber keeps a few changes; a slow one loses the oldest, and `poll` reads the whole state anyway.
        Search { shared: Arc::default(), changes: Arc::new(Bus::new(16)), pictures: Arc::default() }
    }
}

/// How much of a file is read to tell its format ([`Format::sniff`] looks at no more).
const SNIFF_BYTES: u64 = 16;

/// What the search uploads of the photo at `path`: its bytes, read once from one open of the file. See
/// [`photo_bytes`].
fn read_photo(path: &Path) -> Result<Vec<u8>, SearchError> {
    let file = File::open(path).map_err(unreadable)?;
    photo_bytes(file)
}

/// The bytes to upload of the photo `file` holds, decided by its content, not its name. Its first bytes are read and
/// sniffed, and anything but a still image (a video, an unknown file) is refused before the rest is read. JPEG, PNG,
/// WebP, GIF and BMP are sent as they are: the set the search has always taken. A TIFF, which Yandex may refuse, is
/// sent as a JPEG of the same size ([`pg_render::transcode::to_jpeg`]).
fn photo_bytes(mut file: impl Read) -> Result<Vec<u8>, SearchError> {
    let mut bytes = Vec::new();
    file.by_ref().take(SNIFF_BYTES).read_to_end(&mut bytes).map_err(unreadable)?;
    let format = Format::sniff(&bytes).ok_or(SearchError::NotStill)?;
    file.read_to_end(&mut bytes).map_err(unreadable)?;
    match format {
        Format::Jpeg | Format::Png | Format::Webp | Format::Gif | Format::Bmp => Ok(bytes),
        Format::Tiff => pg_render::transcode::to_jpeg(&bytes).map_err(|e| SearchError::Undecodable(format!("{e:#}"))),
    }
}

/// The photo could not be read: the error's text.
fn unreadable(e: io::Error) -> SearchError {
    SearchError::Unreadable(e.to_string())
}

/// A request that failed before any results came.
fn request(e: SourceError) -> SearchError {
    SearchError::Request(e.into())
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A thread that panicked must not stop the next search, so a poisoned lock is still used.
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_status(shared: &Mutex<Shared>, changes: &Bus<()>, status: Status) {
    lock(shared).status = Some(status);
    changes.publish(());
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
    let upload = source.upload(&bytes).map_err(request)?;
    set_status(shared, changes, Status::Working(Phase::OtherSizes));
    let other = source.other_sizes(&upload).map_err(request)?;
    set_status(shared, changes, Status::Working(Phase::Similar));
    let (similar, note) = match source.similar(&upload, DEFAULT_MAX_SIMILAR_PAGES) {
        Ok(list) => {
            let shown = list.items.len();
            let note = list.stopped.map(|error| SimilarNote::Stopped { shown, error: error.into() });
            (list.items, note)
        }
        Err(error) => (Vec::new(), Some(SimilarNote::Failed(error.into()))),
    };
    // The ids are numbered when the search's results are known (`number`).
    let found = Found {
        other: other
            .into_iter()
            .map(|s| Hit {
                id: 0,
                url: s.url,
                width: s.width,
                height: s.height,
                suffix: format!("yandex {}x{}", s.width, s.height),
            })
            .collect(),
        similar: similar
            .into_iter()
            .map(|s| Hit {
                id: 0,
                url: s.image_url,
                width: s.width,
                height: s.height,
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

/// Writes a result's file (kept by the search, or fetched now) into `folder`, named `<stem> (<suffix>)` so it sorts
/// next to the photo ([`write_new`]).
fn save_one(images: &Images, hit: &Hit, stem: &OsStr, folder: &Path) -> Result<(), SaveFailure> {
    let kept = images
        .get(hit.id, &hit.url)
        .map_err(|error| SaveFailure::Download { url: hit.url.clone(), error: error.into() })?;
    write_new(folder, &saved_stem(stem, &hit.suffix), kept.format.extension(), &kept.file.bytes).map(drop).map_err(
        |(path, error)| SaveFailure::Write { url: hit.url.clone(), path: shown(&path), error: error.to_string() },
    )
}

/// What a search found, its note, and the fetcher of its results' files.
type Searched = (Found, Option<SimilarNote>, Arc<dyn Fetch>);

/// The search on its thread: its connection is made first (a recorded session or the client), then the search runs on
/// its source. The fetcher goes with the results, for their files.
fn search(photo: &Path, make: &Factory, shared: &Mutex<Shared>, changes: &Bus<()>) -> Result<Searched, SearchError> {
    let mut connection = make().map_err(request)?;
    let (found, note) = find_similar(photo, &mut *connection.source, shared, changes)?;
    Ok((found, note, connection.fetch))
}

/// The outcome of a save with no search on screen to take its files from: nothing downloaded, every hit failed.
fn save_refused(hits: &[Hit], folder: &Path, error: SourceError) -> Saved {
    let failed = hits.len();
    Saved { folder: shown(folder), written: 0, failed, first_error: Some(SaveFailure::Client(error.into())) }
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
    let mut saved = Saved { folder: shown(folder), written: 0, failed: 0, first_error: None };
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
    /// The results' files of the search on screen, if there is one.
    fn pictures(&self) -> Option<Arc<Images>> {
        self.pictures.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

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
    /// session), its results' grid pictures made by `check`. False when a search or a save is running, or the search
    /// has stopped (nothing changes); an error when its thread couldn't start (nothing runs, and nothing stays busy).
    pub fn start(&self, photo: &Path, make: Factory, check: Check) -> io::Result<bool> {
        let photo = photo.to_path_buf();
        {
            let mut s = lock(&self.shared);
            if s.busy || s.closed {
                return Ok(false);
            }
            s.busy = true;
            s.photo = photo.clone();
            s.status = Some(Status::Working(Phase::Starting));
        }
        self.changes.publish(());
        let shared = self.shared.clone();
        let changes = self.changes.clone();
        let slot = self.pictures.clone();
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
                    let pictures = Images::new(fetch, check, changes.clone());
                    let last = slot.write().unwrap_or_else(PoisonError::into_inner).replace(pictures.clone());
                    // The last search's fetchers stop: its results left the screen.
                    if let Some(last) = last {
                        last.stop();
                    }
                    // A search that ends after the stop keeps its results but fetches nothing.
                    if s.closed {
                        pictures.stop();
                    } else {
                        fetching = Some((pictures, results));
                    }
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
        let dead = self.pictures().map(|pictures| pictures.take_dead()).unwrap_or_default();
        Poll { busy: s.busy, status: s.status.take(), found: s.found.take(), dead }
    }

    /// The grid picture of the result `id` of the search on screen: fetched now if it is not kept yet.
    pub fn image(&self, id: u64) -> Result<Arc<Fetched>, SourceError> {
        self.pictures().ok_or(SourceError::Yandex(YandexError::Unexpected("no search")))?.get_known(id)
    }

    /// No search or save starts from now on, and the fetchers take no more results. Waits up to `wait` for the running
    /// search or save and the fetchers to end; false when one still ran after it.
    pub fn stop(&self, wait: Duration) -> bool {
        lock(&self.shared).closed = true;
        let pictures = self.pictures();
        if let Some(pictures) = &pictures {
            pictures.stop();
        }
        let deadline = Instant::now() + wait;
        loop {
            let ended = !lock(&self.shared).busy && pictures.as_ref().is_none_or(|p| p.idle());
            if ended {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Where the save dialog opens: the folder saved into last (`yandex_folder`) while it exists, or else the photo's
    /// folder. None when there is neither. In the form that reaches the folder exactly (`disk_path`); the dialog starts
    /// at the nearest folder the shell can name (`pg_platform::picker::start_folder`).
    pub fn start_folder(&self, prefs: &PrefStore) -> Option<PathBuf> {
        let last = prefs.get().yandex_folder.map(|folder| disk_path(PATH_RULES, &folder));
        if let Some(last) = last.filter(|last| last.is_dir()) {
            return Some(last);
        }
        let photo = lock(&self.shared).photo.clone();
        photo.parent().map(Path::to_path_buf)
    }

    /// Save the picked `hits` into `folder`, one file each, named after the photo, from the files the search kept. The
    /// folder is remembered in the preferences for the next save, whether or not every save works. False when nothing
    /// is picked, a search or a save is running, or the search has stopped (nothing changes); an error when its thread couldn't start (nothing
    /// is written, and nothing stays busy). The save's outcome comes from [`Search::poll`].
    pub fn save(&self, prefs: &PrefStore, hits: Vec<Hit>, folder: PathBuf) -> io::Result<bool> {
        if hits.is_empty() {
            return Ok(false);
        }
        let stem = {
            let mut s = lock(&self.shared);
            if s.busy || s.closed {
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
        let pictures = self.pictures();
        self.spawn_worker("yandex save", move || {
            // A panic here is logged (the hook), and the save ends with a status, so `busy` is always cleared.
            let status = match pg_jobs::panics::catch(AssertUnwindSafe(|| match pictures {
                Some(pictures) => save_all(&hits, &stem, &folder, &pictures, &shared, &changes),
                None => save_refused(&hits, &folder, YandexError::Unexpected("no search on screen").into()),
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
    fn a_stopped_search_starts_nothing() {
        let search = Search::default();
        assert!(search.stop(Duration::ZERO), "nothing ran");
        let check: Check = Arc::new(|_| Ok(Vec::new()));
        let make: Factory = Arc::new(|| Err(YandexError::Unexpected("not called").into()));
        assert!(matches!(search.start(Path::new("photo.jpg"), make, check), Ok(false)));
        assert!(!lock(&search.shared).busy);
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
        let folder = pg_testkit::run_folder("scratch_app", "yandex-missing").unwrap();
        assert!(matches!(read_photo(&folder.join("gone.jpg")), Err(SearchError::Unreadable(_))));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
