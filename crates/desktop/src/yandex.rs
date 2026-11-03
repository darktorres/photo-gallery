//! Reverse image search on Yandex, from the viewer: the other sizes of a photo and the images that look like it
//! (`pg_core::yandex`). The search and the downloads run on background threads; QML calls `poll()` while `busy`.
//! The results are items by index, as the grid's are (`GalleryModel`): the other sizes first (`otherCount` of them),
//! then the similar images (`similarCount`). The picks to save are kept here too. Nothing here touches the catalog:
//! the files go into the folder the user picks, named after the photo (`IMG_1234 (yandex 1920x1080).jpg`), and that
//! folder is remembered (`Prefs::yandex_folder`) for the next save.

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

        /// Pick a result to be saved, or unpick it. It becomes the anchor of the next `pickTo`.
        #[qinvokable]
        #[cxx_name = "togglePick"]
        fn toggle_pick(self: Pin<&mut YandexSearch>, item: i32);

        /// Shift-click: every result from the anchor (the last one toggled) to `item` takes the anchor's state, picked
        /// or not. Without an anchor, toggles `item`.
        #[qinvokable]
        #[cxx_name = "pickTo"]
        fn pick_to(self: Pin<&mut YandexSearch>, item: i32);

        /// Pick (or unpick) the `count` results from `first`: a group's Select all and Select none.
        #[qinvokable]
        #[cxx_name = "setPicked"]
        fn set_picked(self: Pin<&mut YandexSearch>, first: i32, count: i32, picked: bool);

        /// How many of the `count` results from `first` are picked.
        #[qinvokable]
        #[cxx_name = "pickedIn"]
        fn picked_in(self: &YandexSearch, first: i32, count: i32) -> i32;

        /// Where the save dialog opens: the folder saved into last, while it exists, or else the photo's folder.
        #[qinvokable]
        #[cxx_name = "startFolder"]
        fn start_folder(self: &YandexSearch) -> QUrl;

        /// Save the picked results into `folder`, a file URL.
        #[qinvokable]
        fn download(self: Pin<&mut YandexSearch>, folder: &QUrl);
    }
}

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use pg_core::yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, YandexError};

use crate::{panic_log, settings};

/// The status line when a search or a save stops at an internal error (the panic is in the log).
const INTERNAL_STOP: &str = "Stopped by an internal error; the details are in gallery.log.";

/// One result as the panel shows and saves it.
#[derive(Clone)]
struct Hit {
    url: String,
    thumb: String,
    width: i32,
    height: i32,
    /// What follows the photo's name in the saved file's name, inside the brackets: `yandex 1920x1080`.
    suffix: String,
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
    /// The photo searched for, a local path: the saved files are named after it.
    photo: String,
    /// The other sizes, then the similar images.
    hits: Vec<Hit>,
    picked: Vec<bool>,
    /// The result toggled last, where a Shift-click range starts.
    anchor: Option<usize>,
}

impl YandexSearchRust {
    fn hit(&self, item: i32) -> Option<&Hit> {
        usize::try_from(item).ok().and_then(|i| self.hits.get(i))
    }

    /// The results `first..first + count` that exist.
    fn range(&self, first: i32, count: i32) -> std::ops::Range<usize> {
        let len = self.picked.len();
        let start = usize::try_from(first).unwrap_or(0).min(len);
        start..start.saturating_add(usize::try_from(count).unwrap_or(0)).min(len)
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
        Ok(list) => {
            let note = list
                .stopped
                .map(|e| format!("Similar images stopped after {} results: {}", list.items.len(), explain(e)));
            (list.items, note)
        }
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

/// `file:///C:/a/b`: a local path as a URL, for the folder dialog. `%`, `#` and `?` are escaped, or the URL would read
/// them as an escape, a fragment and a query.
fn file_url(path: &Path) -> String {
    let mut url = "file:///".to_string();
    for c in path.to_string_lossy().chars() {
        match c {
            '\\' => url.push('/'),
            '%' => url.push_str("%25"),
            '#' => url.push_str("%23"),
            '?' => url.push_str("%3F"),
            c => url.push(c),
        }
    }
    url
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

/// Downloads every hit into `folder`, one at a time (the client paces nothing here; these are CDN files), each named
/// `<stem> (<suffix>)` so they sort next to the photo.
fn save_all(hits: &[Hit], stem: &str, folder: &Path, shared: &Mutex<Shared>) -> Saved {
    let client = match Client::new() {
        Ok(client) => client,
        Err(e) => return Saved { written: 0, failed: hits.len(), first_error: Some(explain(e)) },
    };
    let mut saved = Saved { written: 0, failed: 0, first_error: None };
    for (i, hit) in hits.iter().enumerate() {
        set_status(shared, format!("Saving {} of {}…", i + 1, hits.len()));
        let result = client.download(&hit.url).map_err(|e| e.to_string()).and_then(|file| {
            let path = unique_path(folder, &sanitize(&format!("{stem} ({})", hit.suffix)), file.extension);
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
        self.as_mut().rust_mut().photo = path.clone();
        self.as_mut().show(Vec::new(), Vec::new());
        thread::spawn(move || {
            // A panic here is logged, and the search ends with a status line, so `busy` is always cleared.
            let outcome = panic_log::catch(AssertUnwindSafe(|| find_similar(&path, &shared)))
                .unwrap_or_else(|_| Err(INTERNAL_STOP.to_string()));
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
            this.anchor = None;
        }
        self.as_mut().set_other_count(other_count);
        self.as_mut().set_similar_count(similar_count);
        self.picks_changed();
    }

    /// After the picks change: their count, and a new revision for the bindings.
    fn picks_changed(mut self: Pin<&mut Self>) {
        let picked_count = self.picked.iter().filter(|&&p| p).count() as i32;
        self.as_mut().set_picked_count(picked_count);
        let revision = self.revision + 1;
        self.set_revision(revision);
    }

    fn poll(mut self: Pin<&mut Self>) {
        let (busy, status, found) = {
            let mut s = lock(&self.shared);
            (s.busy, s.status.clone(), s.found.take())
        };
        // The results first: whatever watches `busy` going off finds them in place.
        if let Some(found) = found {
            self.as_mut().show(found.other, found.similar);
        }
        if self.status.to_string() != status {
            self.as_mut().set_status(QString::from(&status));
        }
        self.set_busy(busy);
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
        {
            let mut this = self.as_mut().rust_mut();
            this.picked[i] = !this.picked[i];
            this.anchor = Some(i);
        }
        self.picks_changed();
    }

    fn pick_to(mut self: Pin<&mut Self>, item: i32) {
        let Some(i) = usize::try_from(item).ok().filter(|&i| i < self.picked.len()) else { return };
        let Some(anchor) = self.anchor.filter(|&a| a < self.picked.len()) else { return self.toggle_pick(item) };
        {
            let mut this = self.as_mut().rust_mut();
            let value = this.picked[anchor];
            this.picked[anchor.min(i)..=anchor.max(i)].fill(value);
        }
        self.picks_changed();
    }

    fn set_picked(mut self: Pin<&mut Self>, first: i32, count: i32, picked: bool) {
        let range = self.range(first, count);
        self.as_mut().rust_mut().picked[range].fill(picked);
        self.picks_changed();
    }

    fn picked_in(&self, first: i32, count: i32) -> i32 {
        self.picked[self.range(first, count)].iter().filter(|&&p| p).count() as i32
    }

    fn start_folder(&self) -> QUrl {
        let last = PathBuf::from(settings::prefs().yandex_folder);
        let folder = if !last.as_os_str().is_empty() && last.is_dir() {
            Some(last)
        } else {
            Path::new(&self.photo).parent().map(Path::to_path_buf)
        };
        folder.map_or_else(QUrl::default, |f| QUrl::from(&file_url(&f)))
    }

    fn download(mut self: Pin<&mut Self>, folder: &QUrl) {
        let folder = folder.to_local_file().map_or_else(|| folder.to_qstring().to_string(), |p| p.to_string());
        let hits: Vec<Hit> =
            self.hits.iter().zip(&self.picked).filter(|&(_, &picked)| picked).map(|(h, _)| h.clone()).collect();
        if hits.is_empty() {
            return;
        }
        let stem =
            Path::new(&self.photo).file_stem().map_or_else(|| "image".into(), |s| s.to_string_lossy().into_owned());
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
        // Remembered once chosen: the user went there, whether or not every download works.
        let chosen = folder.clone();
        settings::update(|p| p.yandex_folder = chosen);
        thread::spawn(move || {
            // A panic here is logged, and the save ends with a status line, so `busy` is always cleared.
            let status = match panic_log::catch(AssertUnwindSafe(|| {
                save_all(&hits, &stem, Path::new(&folder), &shared)
            })) {
                Ok(saved) => match saved.first_error {
                    None => {
                        format!("Saved {} file{} to {folder}", saved.written, if saved.written == 1 { "" } else { "s" })
                    }
                    Some(first) => {
                        format!("Saved {}, {} failed. First failure: {first}", saved.written, saved.failed)
                    }
                },
                Err(_) => INTERNAL_STOP.to_string(),
            };
            let mut s = lock(&shared);
            s.status = status;
            s.busy = false;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_escapes_what_a_url_would_misread() {
        assert_eq!(file_url(Path::new(r"C:\a b\c")), "file:///C:/a b/c");
        assert_eq!(file_url(Path::new(r"D:\100% #1?\x")), "file:///D:/100%25 %231%3F/x");
    }

    #[test]
    fn saved_names_follow_the_photo() {
        let folder = std::env::temp_dir();
        let name = sanitize(&format!("{} ({})", "IMG_1234", "yandex 1920x1080"));
        assert_eq!(unique_path(&folder, &name, "jpg"), folder.join("IMG_1234 (yandex 1920x1080).jpg"));
    }
}
