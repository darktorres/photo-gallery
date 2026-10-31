//! Reverse image search on Yandex, from the viewer: the other sizes of a photo and the images that look like it
//! (`pg_core::yandex`). The search and the downloads run on background threads; QML calls `poll()` while `busy` and
//! reads the results as JSON. Nothing here touches the catalog: the files go into the folder the user picks.

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
        #[qproperty(bool, busy)]
        #[qproperty(QString, status)]
        /// The latest search as JSON: `{"otherSizes": [...], "similar": [...]}`. Set once, when a search ends.
        #[qproperty(QString, results)]
        type YandexSearch = super::YandexSearchRust;
    }

    unsafe extern "RustQt" {
        /// Upload the photo at the local file `path` and read its other sizes and similar images.
        #[qinvokable]
        fn search(self: Pin<&mut YandexSearch>, path: &QString);

        /// Save `items` (JSON: `[{"url": …, "name": …}]`) into `folder`, a file URL. Names get an extension.
        #[qinvokable]
        fn download(self: Pin<&mut YandexSearch>, items: &QString, folder: &QUrl);

        /// Copy the background state into the properties. QML calls this while `busy`.
        #[qinvokable]
        fn poll(self: Pin<&mut YandexSearch>);
    }
}

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use cxx_qt_lib::{QString, QUrl};
use pg_core::yandex::{Client, DEFAULT_MAX_SIMILAR_PAGES, YandexError};
use serde::Deserialize;
use serde_json::json;

/// What the background threads share with `poll()`.
#[derive(Default)]
struct Shared {
    busy: bool,
    status: String,
    results: Option<String>,
}

#[derive(Default)]
pub struct YandexSearchRust {
    busy: bool,
    status: QString,
    results: QString,
    shared: Arc<Mutex<Shared>>,
}

#[derive(Deserialize)]
struct DownloadItem {
    url: String,
    name: String,
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
        YandexError::Captcha => {
            "Yandex asked for a captcha. Open yandex.com in a browser, solve it, then search again.".to_string()
        }
        other => other.to_string(),
    }
}

/// The Content-Type Yandex's upload expects, from the file's extension. Videos and anything else are refused.
fn content_type_for(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

/// The search, on its own thread: upload, other sizes, similar. Returns the results as JSON, or the message to show.
fn find_similar(path: &str, shared: &Mutex<Shared>) -> Result<String, String> {
    let content_type = content_type_for(path)
        .ok_or_else(|| "Yandex search takes still images (JPEG, PNG, WebP, GIF or BMP).".to_string())?;
    let bytes = std::fs::read(path).map_err(|e| format!("Can't read the photo: {e}"))?;
    let mut client = Client::new();
    set_status(shared, "Uploading the photo…");
    let upload = client.upload(&bytes, content_type).map_err(explain)?;
    set_status(shared, "Reading the other sizes…");
    let other = client.other_sizes(&upload).map_err(explain)?;
    set_status(shared, "Reading similar images…");
    let similar = client.similar(&upload, DEFAULT_MAX_SIMILAR_PAGES).map_err(explain)?;
    Ok(json!({
        "otherSizes": other
            .iter()
            .map(|s| json!({"group": s.group, "width": s.width, "height": s.height, "url": s.url}))
            .collect::<Vec<_>>(),
        "similar": similar
            .iter()
            .map(|s| json!({"pos": s.pos, "width": s.width, "height": s.height,
                            "imageUrl": s.image_url, "thumbUrl": s.thumb_url}))
            .collect::<Vec<_>>(),
    })
    .to_string())
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

/// Downloads every item into `folder`, one at a time (the client paces nothing here; these are CDN files).
fn save_all(items: &[DownloadItem], folder: &Path, shared: &Mutex<Shared>) -> Saved {
    let client = Client::new();
    let mut saved = Saved { written: 0, failed: 0, first_error: None };
    for (i, item) in items.iter().enumerate() {
        set_status(shared, format!("Saving {} of {}…", i + 1, items.len()));
        let result = client.download(&item.url).map_err(|e| e.to_string()).and_then(|file| {
            let path = unique_path(folder, &sanitize(&item.name), file.extension);
            std::fs::write(&path, &file.bytes).map_err(|e| format!("{}: {e}", path.display()))
        });
        match result {
            Ok(()) => saved.written += 1,
            Err(e) => {
                saved.failed += 1;
                saved.first_error.get_or_insert(format!("{}: {e}", item.url));
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
        self.as_mut().set_results(QString::default());
        thread::spawn(move || {
            let outcome = find_similar(&path, &shared);
            let mut s = lock(&shared);
            match outcome {
                Ok(json) => {
                    s.results = Some(json);
                    s.status = String::new();
                }
                Err(message) => s.status = message,
            }
            // Same lock as the results, so `poll()` never sees the search end before its results.
            s.busy = false;
        });
    }

    fn download(mut self: Pin<&mut Self>, items: &QString, folder: &QUrl) {
        let folder = folder.to_local_file().map_or_else(|| folder.to_qstring().to_string(), |p| p.to_string());
        let items: Vec<DownloadItem> = match serde_json::from_str(&items.to_string()) {
            Ok(items) => items,
            Err(e) => return self.as_mut().set_status(QString::from(&format!("Nothing to save: {e}"))),
        };
        let shared = self.shared.clone();
        {
            let mut s = lock(&shared);
            if s.busy {
                return;
            }
            s.busy = true;
            s.status = format!("Saving {} files…", items.len());
        }
        thread::spawn(move || {
            let saved = save_all(&items, Path::new(&folder), &shared);
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

    fn poll(mut self: Pin<&mut Self>) {
        let (busy, status, results) = {
            let mut s = lock(&self.shared);
            (s.busy, s.status.clone(), s.results.take())
        };
        self.as_mut().set_busy(busy);
        if self.status.to_string() != status {
            self.as_mut().set_status(QString::from(&status));
        }
        if let Some(json) = results {
            self.as_mut().set_results(QString::from(&json));
        }
    }
}
