//! Reverse image search on Yandex (`pg_yandex`): the other sizes of a photo and the images that look like it, and the
//! saving of the results the user picks. The search and the saves run on background threads. The front end subscribes
//! ([`Search::subscribe`]) and calls [`Search::poll`] on each change, and shows what comes back ([`Poll::busy`] says
//! whether one is still running). Results and words are typed values: nothing here
//! formats text or prints. The files go into the folder the user picks, named after the photo (`IMG_1234 (yandex
//! 1920x1080).jpg`), and that folder is remembered in the preferences (`yandex_folder`) for the next save.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use pg_jobs::events::{Bus, Subscription};
pub use pg_yandex::YandexError;
use pg_yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, Download, OtherSize, Similar, Upload};

use crate::app::App;

pub mod replay;
pub use replay::{Recording, Replay};

/// Where a search or a save gets its Yandex calls: the real client, or a recorded session that stands in for it.
/// Each search and each save makes its own, on its thread.
pub trait Source {
    fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError>;
    fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, YandexError>;
    fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, YandexError>;
    fn download(&self, url: &str) -> Result<Download, YandexError>;
}

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

    fn download(&self, url: &str) -> Result<Download, YandexError> {
        Client::download(self, url)
    }
}

/// Makes the source for one search or one save. The front end passes [`client()`], or [`replay()`] when it is given a
/// recorded session explicitly.
pub type Factory = Arc<dyn Fn() -> Result<Box<dyn Source + Send>, YandexError> + Send + Sync>;

/// The real Yandex client, made for each search and each save. The default.
pub fn client() -> Factory {
    Arc::new(|| -> Result<Box<dyn Source + Send>, YandexError> {
        let source: Box<dyn Source + Send> = Box::new(Client::new()?);
        Ok(source)
    })
}

/// The recorded session in `dir` (the folder's layout is in the `replay` module), standing in for the client. It is read once, here; when it
/// can't be read, every search and save fails with the reason, so nothing is sent to Yandex in its place.
pub fn replay(dir: &Path) -> Factory {
    let recording = Recording::read(dir).map(Arc::new);
    Arc::new(move || match &recording {
        Ok(recording) => {
            let source: Box<dyn Source + Send> = Box::new(Replay::new(recording.clone()));
            Ok(source)
        }
        Err(why) => Err(YandexError::Recorded(why.clone())),
    })
}

/// One result as the panel shows and saves it.
#[derive(Clone, Debug)]
pub struct Hit {
    pub url: String,
    pub thumb: String,
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
    /// Refused before anything is sent: only still images are searched.
    NotStill,
    /// The photo could not be read from disk.
    Unreadable(std::io::Error),
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
}

/// What the background threads share with [`Search::poll`].
#[derive(Default)]
struct Shared {
    busy: bool,
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

/// Still images only: videos and anything else are refused before anything is sent.
fn is_still_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"))
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
    if !is_still_image(photo) {
        return Err(SearchError::NotStill);
    }
    let bytes = std::fs::read(photo).map_err(SearchError::Unreadable)?;
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
    let found = Found {
        other: other
            .into_iter()
            .map(|s| Hit {
                thumb: s.url.clone(),
                url: s.url,
                width: dimension(s.width),
                height: dimension(s.height),
                suffix: format!("yandex {}x{}", s.width, s.height),
            })
            .collect(),
        similar: similar
            .into_iter()
            .map(|s| Hit {
                thumb: s.thumb_url.unwrap_or_else(|| s.image_url.clone()),
                url: s.image_url,
                width: dimension(s.width),
                height: dimension(s.height),
                suffix: format!("yandex similar {}", s.pos + 1),
            })
            .collect(),
    };
    Ok((found, note))
}

/// A file name without characters that Windows refuses in names.
fn sanitize(name: &str) -> String {
    let clean: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let clean = clean.trim();
    if clean.is_empty() { "image".to_string() } else { clean.to_string() }
}

/// `folder/stem.ext`, or with ` (2)`, ` (3)`… before the extension when that name is taken.
fn unique_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = folder.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// The file-name stem of the photo: `IMG_1234` for `IMG_1234.jpg`.
fn stem_of(photo: &Path) -> String {
    photo.file_stem().map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned())
}

/// Downloads one hit into `folder`, named `<stem> (<suffix>)` so it sorts next to the photo.
fn save_one<S: Source + ?Sized>(source: &S, hit: &Hit, stem: &str, folder: &Path) -> Result<(), SaveFailure> {
    let file = source.download(&hit.url).map_err(|error| SaveFailure::Download { url: hit.url.clone(), error })?;
    let path = unique_path(folder, &sanitize(&format!("{stem} ({})", hit.suffix)), file.format.extension());
    std::fs::write(&path, &file.bytes).map_err(|error| SaveFailure::Write { url: hit.url.clone(), path, error })
}

/// The search on its thread: its source is made first (a recorded session or the client), then the search runs on it.
fn search(
    photo: &Path,
    make: &Factory,
    shared: &Mutex<Shared>,
    changes: &Bus<()>,
) -> Result<(Found, Option<SimilarNote>), SearchError> {
    let mut source = make().map_err(SearchError::Request)?;
    find_similar(photo, &mut *source, shared, changes)
}

/// The outcome of a save whose source could not be made: nothing downloaded, every hit failed.
fn save_refused(hits: &[Hit], folder: &Path, error: YandexError) -> Saved {
    let failed = hits.len();
    Saved { folder: folder.to_path_buf(), written: 0, failed, first_error: Some(SaveFailure::Client(error)) }
}

/// Downloads every hit into `folder`, one at a time (these are CDN files, so the client paces nothing here).
fn save_all<S: Source + ?Sized>(
    hits: &[Hit],
    stem: &str,
    folder: &Path,
    source: &S,
    shared: &Mutex<Shared>,
    changes: &Bus<()>,
) -> Saved {
    let mut saved = Saved { folder: folder.to_path_buf(), written: 0, failed: 0, first_error: None };
    for (i, hit) in hits.iter().enumerate() {
        set_status(shared, changes, Status::Working(Phase::SavingFile { n: i + 1, of: hits.len() }));
        match save_one(source, hit, stem, folder) {
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
    /// Start a search for the photo at `photo`, a local path, with the source `make` gives (the client, or a recorded
    /// session). False when a search or a save is running (nothing changes).
    pub fn start(&self, photo: &Path, make: Factory) -> bool {
        let photo = photo.to_path_buf();
        {
            let mut s = lock(&self.shared);
            if s.busy {
                return false;
            }
            s.busy = true;
            s.photo = photo.clone();
            s.status = Some(Status::Working(Phase::Starting));
        }
        self.changes.publish(());
        let shared = self.shared.clone();
        let changes = self.changes.clone();
        thread::spawn(move || {
            // A panic here is logged (the hook), and the search ends with a status, so `busy` is always cleared.
            let outcome = pg_jobs::panics::catch(AssertUnwindSafe(|| search(&photo, &make, &shared, &changes)));
            let mut s = lock(&shared);
            match outcome {
                Ok(Ok((found, note))) => {
                    s.found = Some(found);
                    s.status = Some(Status::Searched(note));
                }
                Ok(Err(e)) => s.status = Some(Status::SearchFailed(e)),
                Err(_) => s.status = Some(Status::Stopped),
            }
            // Same lock as the results, so `poll()` never sees the search end before its results.
            s.busy = false;
            drop(s);
            changes.publish(());
        });
        true
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
        Poll { busy: s.busy, status: s.status.take(), found: s.found.take() }
    }

    /// Where the save dialog opens: the folder saved into last (`yandex_folder`) while it exists, or else the photo's
    /// folder. None when there is neither.
    pub fn start_folder(&self, app: &App) -> Option<PathBuf> {
        let last = PathBuf::from(app.prefs().yandex_folder);
        if !last.as_os_str().is_empty() && last.is_dir() {
            return Some(last);
        }
        let photo = lock(&self.shared).photo.clone();
        photo.parent().map(Path::to_path_buf)
    }

    /// Save the picked `hits` into `folder`, one file each, named after the photo, downloaded from the source `make`
    /// gives. The folder is remembered in the preferences for the next save, whether or not every download works. False
    /// when nothing is picked or a search or a save is running (nothing changes). The save's outcome comes from
    /// [`Search::poll`].
    pub fn save(&self, app: &App, hits: Vec<Hit>, folder: PathBuf, make: Factory) -> bool {
        if hits.is_empty() {
            return false;
        }
        let stem = {
            let mut s = lock(&self.shared);
            if s.busy {
                return false;
            }
            s.busy = true;
            s.status = Some(Status::Working(Phase::Saving { files: hits.len() }));
            stem_of(&s.photo)
        };
        self.changes.publish(());
        // A preferences file that can't be written keeps the folder in memory for this run; the save goes on. The app
        // reports that failure through its notes when it can be reported.
        let chosen = folder.to_string_lossy().into_owned();
        let _ = app.update_prefs(|p| p.yandex_folder = chosen);
        let shared = self.shared.clone();
        let changes = self.changes.clone();
        thread::spawn(move || {
            // A panic here is logged (the hook), and the save ends with a status, so `busy` is always cleared.
            let status = match pg_jobs::panics::catch(AssertUnwindSafe(|| match make() {
                Ok(source) => save_all(&hits, &stem, &folder, &*source, &shared, &changes),
                Err(e) => save_refused(&hits, &folder, e),
            })) {
                Ok(saved) => Status::Saved(saved),
                Err(_) => Status::Stopped,
            };
            let mut s = lock(&shared);
            s.status = Some(status);
            s.busy = false;
            drop(s);
            changes.publish(());
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_names_follow_the_photo() {
        let folder = std::env::temp_dir();
        let name = sanitize(&format!("{} ({})", "IMG_1234", "yandex 1920x1080"));
        assert_eq!(unique_path(&folder, &name, "jpg"), folder.join("IMG_1234 (yandex 1920x1080).jpg"));
    }
}
