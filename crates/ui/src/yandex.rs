//! Reverse image search on Yandex, from the viewer: the other sizes of a photo and the images that look like it
//! (`pg_core::yandex`). The search and the downloads run on background threads; QML calls `poll()` while `busy`.
//! The results are items by index, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). The picks to save are kept here too. Nothing here touches the catalog:
//! the files go into the folder the user picks.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, busy, READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(QString, status, READ, WRITE, NOTIFY, FINAL)]
        /// Results of the latest search: the other sizes, then the similar images.
        #[qproperty(i32, other_count, cxx_name = "otherCount", READ, WRITE, NOTIFY, FINAL)]
        #[qproperty(i32, similar_count, cxx_name = "similarCount", READ, WRITE, NOTIFY, FINAL)]
        /// Results picked to be saved.
        #[qproperty(i32, picked_count, cxx_name = "pickedCount", READ, WRITE, NOTIFY, FINAL)]
        /// Changes whenever the results or the picks do: for bindings that call the item functions.
        #[qproperty(i32, revision, READ, WRITE, NOTIFY, FINAL)]
        type YandexSearch = super::YandexSearchRust;
    }

    unsafe extern "RustQt" {
        /// Upload the photo at the local file `path` and read its other sizes and similar images.
        #[qinvokable]
        fn search(self: Pin<&mut YandexSearch>, path: &QString);

        /// Copy the background state into the properties. QML calls this while `busy`.
        #[qinvokable]
        fn poll(self: Pin<&mut YandexSearch>);

        /// A result's full-size file.
        #[qinvokable]
        fn url(self: &YandexSearch, item: i32) -> QString;

        /// A picture of the result to show: Yandex's preview for a similar image, the file itself for another size.
        #[qinvokable]
        fn thumb(self: &YandexSearch, item: i32) -> QString;

        /// Its size in pixels, as Yandex gives it; 0 when it doesn't.
        #[qinvokable]
        fn width(self: &YandexSearch, item: i32) -> i32;

        #[qinvokable]
        fn height(self: &YandexSearch, item: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "isPicked"]
        fn is_picked(self: &YandexSearch, item: i32) -> bool;

        /// Pick a result to be saved, or unpick it.
        #[qinvokable]
        #[cxx_name = "togglePick"]
        fn toggle_pick(self: Pin<&mut YandexSearch>, item: i32);

        /// Save the picked results into `folder`, a file URL.
        #[qinvokable]
        fn download(self: Pin<&mut YandexSearch>, folder: &QUrl);
    }
}

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use pg_core::yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, YandexError};

/// One result as the panel shows and saves it.
#[derive(Clone)]
struct Hit {
    url: String,
    thumb: String,
    width: i32,
    height: i32,
    /// The file name it is saved under, before the extension.
    name: String,
}

/// What a finished search found.
struct Found {
    other: Vec<Hit>,
    similar: Vec<Hit>,
}

/// What the background threads share with `poll()`.
#[derive(Default)]
struct Shared {
    busy: bool,
    status: String,
    found: Option<Found>,
}

#[derive(Default)]
pub struct YandexSearchRust {
    busy: bool,
    status: QString,
    other_count: i32,
    similar_count: i32,
    picked_count: i32,
    revision: i32,
    shared: Arc<Mutex<Shared>>,
    /// The other sizes, then the similar images.
    hits: Vec<Hit>,
    picked: Vec<bool>,
}

impl YandexSearchRust {
    fn hit(&self, item: i32) -> Option<&Hit> {
        usize::try_from(item).ok().and_then(|i| self.hits.get(i))
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A thread that panicked must not stop the next search, so a poisoned lock is still used.
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_status(shared: &Mutex<Shared>, text: impl Into<String>) {
    lock(shared).status = text.into();
}

/// What the user reads when a request fails.
fn explain(e: YandexError) -> String {
    match e {
        // Solving it in a browser doesn't help: the app doesn't share the browser's cookies.
        YandexError::Captcha => "Yandex asked for a captcha, which the app can't answer. Try again later.".to_string(),
        other => other.to_string(),
    }
}

/// Still images only: videos and anything else are refused before anything is sent.
fn is_still_image(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"))
}

fn dimension(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The search, on its own thread: upload, other sizes, similar. Returns what it found, with a note when the similar
/// images failed (the other sizes are still shown, as the crawler keeps them), or the message to show.
fn find_similar(path: &str, shared: &Mutex<Shared>) -> Result<(Found, Option<String>), String> {
    if !is_still_image(path) {
        return Err("Yandex search takes still images (JPEG, PNG, WebP, GIF or BMP).".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Can't read the photo: {e}"))?;
    let mut client = Client::new().map_err(explain)?;
    set_status(shared, "Uploading the photo…");
    let upload = client.upload(&bytes).map_err(explain)?;
    set_status(shared, "Reading the other sizes…");
    let other = client.other_sizes(&upload).map_err(explain)?;
    set_status(shared, "Reading similar images…");
    let (similar, note) = match client.similar(&upload, DEFAULT_MAX_SIMILAR_PAGES) {
        Ok(similar) => (similar, None),
        Err(e) => (Vec::new(), Some(format!("Similar images failed: {}", explain(e)))),
    };
    let found = Found {
        other: other
            .into_iter()
            .map(|s| Hit {
                thumb: s.url.clone(),
                url: s.url,
                width: dimension(s.width),
                height: dimension(s.height),
                name: format!("size {}x{}", s.width, s.height),
            })
            .collect(),
        similar: similar
            .into_iter()
            .map(|s| Hit {
                thumb: s.thumb_url.unwrap_or_else(|| s.image_url.clone()),
                url: s.image_url,
                width: dimension(s.width),
                height: dimension(s.height),
                name: format!("similar {}", s.pos + 1),
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

/// What `save_all` did: how many files were written, how many failed, and the first failure's reason.
struct Saved {
    written: usize,
    failed: usize,
    first_error: Option<String>,
}

/// Downloads every hit into `folder`, one at a time (the client paces nothing here; these are CDN files).
fn save_all(hits: &[Hit], folder: &Path, shared: &Mutex<Shared>) -> Saved {
    let client = match Client::new() {
        Ok(client) => client,
        Err(e) => return Saved { written: 0, failed: hits.len(), first_error: Some(explain(e)) },
    };
    let mut saved = Saved { written: 0, failed: 0, first_error: None };
    for (i, hit) in hits.iter().enumerate() {
        set_status(shared, format!("Saving {} of {}…", i + 1, hits.len()));
        let result = client.download(&hit.url).map_err(|e| e.to_string()).and_then(|file| {
            let path = unique_path(folder, &sanitize(&hit.name), file.extension);
            std::fs::write(&path, &file.bytes).map_err(|e| format!("{}: {e}", path.display()))
        });
        match result {
            Ok(()) => saved.written += 1,
            Err(e) => {
                saved.failed += 1;
                saved.first_error.get_or_insert(format!("{}: {e}", hit.url));
            }
        }
    }
    saved
}

impl qobject::YandexSearch {
    fn search(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let shared = self.shared.clone();
        {
            let mut s = lock(&shared);
            if s.busy {
                return;
            }
            s.busy = true;
            s.status = "Starting…".to_string();
        }
        // QML runs its poll timer while the `busy` property is set, and only poll() sets it: set it here too.
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from("Starting…"));
        self.as_mut().show(Vec::new(), Vec::new());
        thread::spawn(move || {
            let outcome = find_similar(&path, &shared);
            let mut s = lock(&shared);
            match outcome {
                Ok((found, note)) => {
                    s.found = Some(found);
                    s.status = note.unwrap_or_default();
                }
                Err(message) => s.status = message,
            }
            // Same lock as the results, so `poll()` never sees the search end before its results.
            s.busy = false;
        });
    }

    /// Show these results, none picked.
    fn show(mut self: Pin<&mut Self>, other: Vec<Hit>, similar: Vec<Hit>) {
        let (other_count, similar_count) = (other.len() as i32, similar.len() as i32);
        {
            let mut this = self.as_mut().rust_mut();
            this.hits = other.into_iter().chain(similar).collect();
            this.picked = vec![false; this.hits.len()];
        }
        self.as_mut().set_other_count(other_count);
        self.as_mut().set_similar_count(similar_count);
        self.as_mut().set_picked_count(0);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn poll(mut self: Pin<&mut Self>) {
        let (busy, status, found) = {
            let mut s = lock(&self.shared);
            (s.busy, s.status.clone(), s.found.take())
        };
        self.as_mut().set_busy(busy);
        if self.status.to_string() != status {
            self.as_mut().set_status(QString::from(&status));
        }
        if let Some(found) = found {
            self.show(found.other, found.similar);
        }
    }

    fn url(&self, item: i32) -> QString {
        self.hit(item).map_or_else(QString::default, |h| QString::from(&h.url))
    }

    fn thumb(&self, item: i32) -> QString {
        self.hit(item).map_or_else(QString::default, |h| QString::from(&h.thumb))
    }

    fn width(&self, item: i32) -> i32 {
        self.hit(item).map_or(0, |h| h.width)
    }

    fn height(&self, item: i32) -> i32 {
        self.hit(item).map_or(0, |h| h.height)
    }

    fn is_picked(&self, item: i32) -> bool {
        usize::try_from(item).ok().and_then(|i| self.picked.get(i)).copied().unwrap_or(false)
    }

    fn toggle_pick(mut self: Pin<&mut Self>, item: i32) {
        let Some(i) = usize::try_from(item).ok().filter(|&i| i < self.picked.len()) else { return };
        let picked_count = {
            let mut this = self.as_mut().rust_mut();
            this.picked[i] = !this.picked[i];
            this.picked.iter().filter(|&&p| p).count() as i32
        };
        self.as_mut().set_picked_count(picked_count);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn download(mut self: Pin<&mut Self>, folder: &QUrl) {
        let folder = folder.to_local_file().map_or_else(|| folder.to_qstring().to_string(), |p| p.to_string());
        let hits: Vec<Hit> =
            self.hits.iter().zip(&self.picked).filter(|&(_, &picked)| picked).map(|(h, _)| h.clone()).collect();
        if hits.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        let starting = format!("Saving {} files…", hits.len());
        {
            let mut s = lock(&shared);
            if s.busy {
                return;
            }
            s.busy = true;
            s.status = starting.clone();
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_status(QString::from(&starting));
        thread::spawn(move || {
            let saved = save_all(&hits, Path::new(&folder), &shared);
            let mut s = lock(&shared);
            s.status = match saved.first_error {
                None => {
                    format!("Saved {} file{} to {folder}", saved.written, if saved.written == 1 { "" } else { "s" })
                }
                Some(first) => format!("Saved {}, {} failed. First failure: {first}", saved.written, saved.failed),
            };
            s.busy = false;
        });
    }
}
