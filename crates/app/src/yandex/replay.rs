//! A recorded Yandex session, standing in for the client: the offline fixture of `gallery --yandex-fixture=<dir>` (the
//! desktop's command line; see `pg_app::yandex::replay`). It is never the default: a search sends the photo to Yandex,
//! so the fixture is used only when that flag names a folder, and the window says so when it starts.
//!
//! The folder holds the answers of one session, as the client read them:
//!
//! - `upload.json`: the upload's answer (`cbir_id`, `sizes.orig.path`).
//! - `other_sizes.html`: the results page the other sizes are read from.
//! - `similar-0.json`, `similar-1.json`, …: the similar endpoint's pages, in order, read up to the first missing number.
//!   A page whose body is Yandex's captcha page (it contains `showcaptcha`) is a captcha, and the list stops there as
//!   it does with the client.
//! - `downloads.tsv`, optional: one line per result file, the URL, a tab, then the file's name in this folder. The
//!   file's bytes are what the save writes; its format is what they sniff as. A URL that has no line is an error.
//!
//! Nothing is read from the network. The files are read once, when the session is opened.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use pg_yandex::{
    Download, OtherSize, Similar, SimilarPage, Upload, YandexError, is_captcha_page, parse_other_sizes,
    parse_similar_page, parse_upload, read_similar_pages, sniffed,
};

use super::images::Fetch;
use super::source::Source;

/// A recorded session, read into memory.
pub struct Recording {
    upload: String,
    other_sizes: String,
    similar: Vec<String>,
    downloads: BTreeMap<String, Vec<u8>>,
}

impl Recording {
    /// Read the session in `dir`. The error names the file that is missing or can't be read.
    pub fn read(dir: &Path) -> Result<Recording, String> {
        let file = |name: &str| dir.join(name);
        let text = |name: &str| {
            let path = file(name);
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        let upload = text("upload.json")?;
        let other_sizes = text("other_sizes.html")?;
        let mut similar = Vec::new();
        while let Ok(page) = std::fs::read_to_string(file(format!("similar-{}.json", similar.len()).as_str())) {
            similar.push(page);
        }
        let mut downloads = BTreeMap::new();
        if let Ok(list) = std::fs::read_to_string(file("downloads.tsv")) {
            for line in list.lines().filter(|line| !line.trim().is_empty()) {
                let (url, name) = line.split_once('\t').ok_or_else(|| format!("downloads.tsv: no tab in {line:?}"))?;
                if name.contains("..") || name.chars().any(|c| c == '/' || c == '\\') {
                    return Err(format!("downloads.tsv: {name:?} is not a file name in the session folder"));
                }
                let bytes = std::fs::read(file(name)).map_err(|e| format!("{}: {e}", file(name).display()))?;
                downloads.insert(url.to_string(), bytes);
            }
        }
        Ok(Recording { upload, other_sizes, similar, downloads })
    }
}

/// The source that answers from a recorded session. Its calls are the client's, in the same order.
pub struct Replay {
    recording: Arc<Recording>,
}

impl Replay {
    pub fn new(recording: Arc<Recording>) -> Replay {
        Replay { recording }
    }
}

impl Source for Replay {
    fn upload(&mut self, _image: &[u8]) -> Result<Upload, YandexError> {
        parse_upload(&self.recording.upload)
    }

    fn other_sizes(&mut self, _up: &Upload) -> Result<Vec<OtherSize>, YandexError> {
        parse_other_sizes(&self.recording.other_sizes)
    }

    fn similar(&mut self, _up: &Upload, max_pages: u32) -> Result<Similar, YandexError> {
        let recording = &self.recording;
        read_similar_pages(max_pages, |page| match recording.similar.get(pg_model::num::index(page)) {
            // Past the recorded pages the list is over, as an empty page is for the client.
            None => Ok(SimilarPage { items: Vec::new(), has_next: false }),
            Some(body) if is_captcha_page(body) => Err(YandexError::Captcha),
            Some(body) => parse_similar_page(body),
        })
    }
}

/// The recorded result files stand in for the fetcher's downloads.
impl Fetch for Replay {
    fn fetch(&self, url: &str) -> Result<Download, YandexError> {
        let bytes = self
            .recording
            .downloads
            .get(url)
            .ok_or_else(|| YandexError::Recorded(format!("no recorded result file for {url}")))?;
        Ok(Download { bytes: bytes.clone(), format: sniffed(bytes)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yandex::images::Images;
    use crate::yandex::{Hit, Saved, SimilarNote, find_similar, number, save_all};
    use pg_jobs::events::Bus;
    use std::sync::Mutex;

    fn accept(_: &[u8]) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }

    /// A source that records the order of its calls, to check the search's.
    struct Logged {
        inner: Replay,
        calls: Vec<&'static str>,
    }

    impl Source for Logged {
        fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError> {
            self.calls.push("upload");
            self.inner.upload(image)
        }
        fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, YandexError> {
            self.calls.push("other_sizes");
            self.inner.other_sizes(up)
        }
        fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, YandexError> {
            self.calls.push("similar");
            self.inner.similar(up, max_pages)
        }
    }

    /// Writes the session the test searches: two similar items on page 0, a captcha on page 1, and the results' files.
    fn write_session(dir: &Path) {
        let other_sizes = r#"<html><body><div class="Root" id="ImagesApp-Q1" data-state="{&quot;initialState&quot;:{&quot;cbirOtherSizes&quot;:{&quot;items&quot;:{&quot;small_dups&quot;:[{&quot;text&quot;:&quot;714×320&quot;,&quot;url&quot;:&quot;https://i.example/a.jpg&quot;}]}}}}"></div></body></html>"#;
        let similar_0 = r#"{"blocks":[{},{"params":{"adapterData":{"serpList":{"items":{"entities":{
            "a":{"pos":0,"url":"/images/search?img_url=https%3A%2F%2Fa.example%2Fone.png","origWidth":640,"origHeight":480,"image":"//t.example/one.jpg"},
            "b":{"pos":1,"url":"/images/search?img_url=https%3A%2F%2Fb.example%2Ftwo.png","origWidth":300,"origHeight":200}
        }},"navi":{"next":"/images/search?p=1"}}}}}]}"#;
        let upload = r#"{"cbir_id":"CBIR1","sizes":{"orig":{"path":"https://avatars.example/orig"}}}"#;
        let files: [(&str, &[u8]); 6] = [
            ("upload.json", upload.as_bytes()),
            ("other_sizes.html", other_sizes.as_bytes()),
            ("similar-0.json", similar_0.as_bytes()),
            ("similar-1.json", b"<html>showcaptcha</html>"),
            ("one.png", b"\x89PNG\r\n\x1a\nresult one"),
            ("downloads.tsv", b"https://a.example/one.png\tone.png\n"),
        ];
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
    }

    #[test]
    fn a_recorded_search_keeps_what_came_before_a_captcha() {
        let dir = std::env::temp_dir().join(format!("pg-yandex-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_session(&dir);
        let photo = dir.join("IMG_1234.jpg");
        // A JPEG's start, so the photo sniffs as a still; the recording answers, so the bytes are never decoded.
        std::fs::write(&photo, b"\xFF\xD8\xFF not decoded: the recording answers").unwrap();

        let recording = Arc::new(Recording::read(&dir).unwrap());
        let mut source = Logged { inner: Replay::new(recording.clone()), calls: Vec::new() };
        let shared = Mutex::new(Default::default());
        let changes: Arc<Bus<()>> = Arc::new(Bus::new(16));
        let (mut found, note) = find_similar(&photo, &mut source, &shared, &changes).unwrap();
        number(&mut found, 1);

        assert_eq!(source.calls, ["upload", "other_sizes", "similar"]);
        assert_eq!(found.other.len(), 1);
        assert_eq!(found.other[0].suffix, "yandex 714x320");
        let urls: Vec<&str> = found.similar.iter().map(|h| h.url.as_str()).collect();
        assert_eq!(urls, ["https://a.example/one.png", "https://b.example/two.png"]);
        assert!(matches!(note, Some(SimilarNote::Stopped { shown: 2, error: YandexError::Captcha })));

        // The save path writes the recorded bytes of the first result, from the search's images.
        let pictures = Images::new(Arc::new(Replay::new(recording)), Arc::new(accept), changes.clone());
        let folder = dir.join("saved");
        std::fs::create_dir_all(&folder).unwrap();
        let hits: Vec<Hit> = found.similar.into_iter().take(1).collect();
        let saved: Saved = save_all(&hits, std::ffi::OsStr::new("IMG_1234"), &folder, &pictures, &shared, &changes);
        assert_eq!((saved.written, saved.failed), (1, 0));
        let written = std::fs::read(folder.join("IMG_1234 (yandex similar 1).png")).unwrap();
        assert_eq!(written, b"\x89PNG\r\n\x1a\nresult one");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replay_without_its_files_says_so_and_sends_nothing() {
        let dir = std::env::temp_dir().join(format!("pg-yandex-replay-missing-{}", std::process::id()));
        let why = Recording::read(&dir).err().unwrap();
        assert!(why.contains("upload.json"), "{why}");
        // The search reports the reason as its request failure; no client is made.
        let factory = super::super::replay(&dir);
        assert!(matches!(factory(), Err(YandexError::Recorded(_))));
    }
}
