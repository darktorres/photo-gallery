//! The search: a paced client that keeps Yandex's cookies, uploads a photo and reads the two result lists.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ureq::typestate::WithoutBody;

use crate::http::{USER_AGENT, agent, read_response, transfer_timeouts};
use crate::parse::{parse_other_sizes, parse_similar_page, parse_upload, read_similar_pages};
use crate::types::{OtherSize, Similar, Upload, YandexError};

const UPLOAD_URL: &str = "https://yandex.ru/images-apphost/image-download?cbird=111";
const SEARCH_URL: &str = "https://yandex.ru/images/search";

/// Yandex's origin: the cookies of the search are for it (and its subdomains), so they are sent to no other host.
const ORIGIN: &str = "https://yandex.ru/";

/// The similar endpoint's `request` parameter, byte for byte as the crawler's `json.dumps(..., separators=(",", ":"))`
/// writes it: key order and all.
const SIMILAR_REQUEST: &str = r#"{"blocks":[{"block":"extra-content","params":{},"version":2},{"block":{"block":"i-react-ajax-adapter:ajax"},"params":{"type":"ImagesApp","ajaxKey":"serpList/fetch"},"version":2}]}"#;

/// Pause between requests. A burst of automated requests gets a captcha.
const MIN_INTERVAL: Duration = Duration::from_millis(1500);

/// Similar pages fetched when the caller does not choose a limit.
pub const DEFAULT_MAX_SIMILAR_PAGES: u32 = 5;

/// A blocking Yandex client. One client paces its own requests, so callers can use it from a worker thread. Its agent
/// keeps the cookies Yandex sets, as the crawler's `requests.Session` does: a request without the cookies the responses
/// before it gave is a new anonymous client to Yandex, and Yandex answers those with a captcha.
pub struct Client {
    agent: ureq::Agent,
    last_request: Option<Instant>,
}

impl Client {
    /// A client with a cookie jar of its own, which starts with the screen metrics cookie (`yp`) for Yandex's origin.
    pub fn new() -> Result<Self, YandexError> {
        let agent = agent();
        let uri = ureq::http::Uri::from_static(ORIGIN);
        let cookie = ureq::Cookie::parse(yp_cookie(), &uri).map_err(|e| YandexError::Transport(e.to_string()))?;
        agent.cookie_jar_lock().insert(cookie, &uri).map_err(|e| YandexError::Transport(e.to_string()))?;
        Ok(Self { agent, last_request: None })
    }

    /// Uploads an image and returns the ids that the result requests need. The body is the raw file, with no
    /// Content-Type, as the crawler sends it. Its bounds are a transfer's (`transfer_timeouts`), not the 30 s of
    /// the whole search request: a large photo on a slow link takes longer than that.
    pub fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError> {
        self.pace();
        let req = transfer_timeouts(self.agent.post(UPLOAD_URL).header("User-Agent", USER_AGENT));
        parse_upload(&read_response(req.send(image))?)
    }

    /// The same image at other resolutions, largest first. One request.
    pub fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, YandexError> {
        let req = self
            .agent
            .get(SEARCH_URL)
            .query("rpt", "imageview")
            .query("url", &up.orig_url)
            .query("cbir_id", &up.cbir_id)
            .query("family", "none");
        let html = self.send(req)?;
        parse_other_sizes(&html)
    }

    /// Visually similar images. Pages are fetched until one is empty, has no next link, or `max_pages` is reached.
    /// Only a failure on the first page is an error. A later page can still be answered with a captcha (Yandex's
    /// anti-robot check, which the cookies of the earlier pages are meant to satisfy), so a failure on a later page ends
    /// the list with the pages already read, and `stopped` says why.
    pub fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, YandexError> {
        read_similar_pages(max_pages, |page| self.similar_request(up, page).and_then(|body| parse_similar_page(&body)))
    }

    fn similar_request(&mut self, up: &Upload, page: u32) -> Result<String, YandexError> {
        let text = format!("url:\"{}\"", host_and_path(&up.orig_url));
        let req = self
            .agent
            .get(SEARCH_URL)
            .query("format", "json")
            .query("request", SIMILAR_REQUEST)
            .query("cbir_id", &up.cbir_id)
            .query("url", &up.orig_url)
            .query("rpt", "imagelike")
            .query("cbir_page", "similar")
            .query("text", &text)
            .query("family", "none")
            .query("p", page.to_string());
        self.send(req)
    }

    fn send(&mut self, req: ureq::RequestBuilder<WithoutBody>) -> Result<String, YandexError> {
        self.pace();
        let req = req.header("User-Agent", USER_AGENT);
        read_response(req.call())
    }

    fn pace(&mut self) {
        if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            if elapsed < MIN_INTERVAL {
                std::thread::sleep(MIN_INTERVAL - elapsed);
            }
        }
        self.last_request = Some(Instant::now());
    }
}

/// Screen metrics in the shape the browser's own script writes. Expires in a year.
fn yp_cookie() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("yp={}.szm.1:1920x1080:1920x937", now + 31_536_000)
}

/// `host/path` of a URL, without the scheme and the query, as the crawler's `netloc + path`.
fn host_and_path(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['?', '#']).next().unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_parameter_is_host_and_path_without_query() {
        assert_eq!(
            host_and_path("https://avatars.mds.yandex.net/get-images-cbir/1/2/orig"),
            "avatars.mds.yandex.net/get-images-cbir/1/2/orig"
        );
        assert_eq!(host_and_path("https://a.example/x/orig?x=1#f"), "a.example/x/orig");
    }

    #[test]
    fn similar_request_is_the_json_of_the_crawlers_monorepos_search_py() {
        // The same bytes `json.dumps(request_json, separators=(",", ":"))` gives in `yandex-reverse/search.py` of the
        // crawlers monorepo, which is outside this repository.
        let v: serde_json::Value = serde_json::from_str(SIMILAR_REQUEST).unwrap();
        assert_eq!(v["blocks"][1]["params"]["ajaxKey"], "serpList/fetch");
        assert!(SIMILAR_REQUEST.starts_with(r#"{"blocks":[{"block":"extra-content""#));
    }
}
