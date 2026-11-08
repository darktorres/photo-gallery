//! Yandex Images reverse search: upload a photo, then read the two result lists.
//!
//! - **Other sizes:** the same image at other resolutions. It sits in the results page's initial state
//!   (`cbirOtherSizes`), so one request gets all of them.
//! - **Similar:** visually similar images, from the `cbir_page=similar` JSON endpoint, paged with `p`.
//!
//! The similar list is short for a client that sends no `yp` cookie with screen metrics
//! (`<ts>.szm.1:WxH:WxH`). That cookie is what Yandex's own script writes into a browser, so the client sends
//! it too. Nothing else is sent: the Sites list is not fetched, because its pages after the first need an
//! anti-robot cookie, and this client does not try to get one.
//!
//! Endpoints and parsing follow `yandex-reverse/search.py` of the crawlers monorepo, which is outside this repository.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::io::Read;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pg_model::format::Format;
use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::typestate::WithoutBody;

const UPLOAD_URL: &str = "https://yandex.com/images-apphost/image-download?cbird=111";
const SEARCH_URL: &str = "https://yandex.com/images/search";
/// The same User-Agent as the crawlers monorepo's `yandex-reverse/search.py` (outside this repository), which Yandex
/// serves without a captcha.
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36";
/// The similar endpoint's `request` parameter, byte for byte as the crawler's `json.dumps(..., separators=(",", ":"))`
/// writes it: key order and all.
const SIMILAR_REQUEST: &str = r#"{"blocks":[{"block":"extra-content","params":{},"version":2},{"block":{"block":"i-react-ajax-adapter:ajax"},"params":{"type":"ImagesApp","ajaxKey":"serpList/fetch"},"version":2}]}"#;
/// The whole of a search request (the results page, a similar page): small answers, so one bound covers it all.
const TIMEOUT: Duration = Duration::from_secs(30);
/// An upload or a download moves a file of up to `MAX_DOWNLOAD_BYTES`, which a slow link takes minutes over, so its
/// bounds are per phase instead (`transfer_timeouts`): the name lookup and the connection, the wait for the answer's
/// headers, and each read or write, which bounds a stalled transfer...
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// ...and the body as a whole, which bounds one that trickles: 100 MB in 15 minutes is about 110 kB/s.
const BODY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Pause between requests. A burst of automated requests gets a captcha.
const MIN_INTERVAL: Duration = Duration::from_millis(1500);
/// Similar pages fetched when the caller does not choose a limit.
pub const DEFAULT_MAX_SIMILAR_PAGES: u32 = 5;
/// Largest result image `Fetcher::download` accepts.
pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Why a Yandex request failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum YandexError {
    /// A result file that is not an image this app can decode (the reason is what failed).
    #[error("not an image this app can decode: {0}")]
    Undecodable(&'static str),
    /// Yandex asked for an anti-robot check. Nothing is retried; the user has to resolve it in a browser.
    #[error("Yandex asked for a captcha")]
    Captcha,
    /// An HTTP status other than success.
    #[error("Yandex answered HTTP {0}")]
    Status(u16),
    /// A connection, TLS or timeout failure.
    #[error("Yandex request failed: {0}")]
    Transport(String),
    /// A response that did not have the data we read.
    #[error("unexpected Yandex response: {0}")]
    Unexpected(&'static str),
    /// A recorded session (`pg_app::yandex`'s fixture) that can't answer: why. Never made by the client.
    #[error("recorded Yandex session: {0}")]
    Recorded(String),
}

/// The ids Yandex gives an upload. `orig_url` is the uploaded photo on Yandex's CDN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub cbir_id: String,
    pub orig_url: String,
}

/// One image at another resolution. `url` is the full-size file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherSize {
    pub group: String,
    pub width: u32,
    pub height: u32,
    pub url: String,
}

/// One visually similar image. `image_url` is the full-size file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarItem {
    pub pos: u32,
    pub width: u32,
    pub height: u32,
    pub image_url: String,
}

/// One result image downloaded with `Fetcher::download`.
#[derive(Debug)]
pub struct Download {
    pub bytes: Vec<u8>,
    /// The format of the bytes, from the bytes themselves (`Format::sniff`).
    pub format: Format,
}

/// One page of the similar endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarPage {
    pub items: Vec<SimilarItem>,
    pub has_next: bool,
}

/// The similar images read, in order, and why the list ends early if it does.
#[derive(Debug)]
pub struct Similar {
    pub items: Vec<SimilarItem>,
    /// The error that ended the list after its first page: a captcha, a failed request, or a page that isn't what
    /// was read. `None` when the list ended by itself (an empty page, no next page) or at the page limit.
    pub stopped: Option<YandexError>,
}

/// The ids in an upload's answer (JSON), which the result requests need.
pub fn parse_upload(body: &str) -> Result<Upload, YandexError> {
    let v: Value = serde_json::from_str(body).map_err(|_| YandexError::Unexpected("upload is not JSON"))?;
    let cbir_id = v.get("cbir_id").and_then(Value::as_str).ok_or(YandexError::Unexpected("upload has no cbir_id"))?;
    let orig_url = v
        .pointer("/sizes/orig/path")
        .and_then(Value::as_str)
        .ok_or(YandexError::Unexpected("upload has no original URL"))?;
    Ok(Upload { cbir_id: cbir_id.to_string(), orig_url: orig_url.to_string() })
}

/// Whether a response body is Yandex's anti-robot page (a captcha), whatever its status.
pub fn is_captcha_page(body: &str) -> bool {
    body.contains("showcaptcha")
}

/// Reads the pages `fetch` gives, from page 0 up to `max_pages`, as [`Client::similar`] does. A failure on page 0 is
/// the error; one on a later page stops the list with the items read so far.
pub fn read_similar_pages(
    max_pages: u32,
    mut fetch: impl FnMut(u32) -> Result<SimilarPage, YandexError>,
) -> Result<Similar, YandexError> {
    let mut items = Vec::new();
    for page in 0..max_pages {
        let parsed = match fetch(page) {
            Ok(p) => p,
            Err(e) if page > 0 => return Ok(Similar { items, stopped: Some(e) }),
            Err(e) => return Err(e),
        };
        if parsed.items.is_empty() {
            break;
        }
        items.extend(parsed.items);
        if !parsed.has_next {
            break;
        }
    }
    Ok(Similar { items, stopped: None })
}

/// A blocking Yandex client. One client paces its own requests, so callers can use it from a worker thread.
pub struct Client {
    agent: ureq::Agent,
    yp: String,
    last_request: Option<Instant>,
}

/// The HTTP agent of the search and of the result files: statuses are handed back to the caller (a captcha page's
/// body says what it is), and TLS is the system's: native-tls on the OS's roots (the platform verifier). The default
/// roots (`WebPki`) are the bundled Mozilla list with the system's store off, and a chain the system builds for a site
/// (i.ytimg.com's) then fails: "unable to find any user-specified roots in the final cert chain".
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(
            TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
        )
        .build();
    ureq::Agent::new_with_config(config)
}

impl Client {
    pub fn new() -> Self {
        Self { agent: agent(), yp: yp_cookie(), last_request: None }
    }

    /// Uploads an image and returns the ids that the result requests need. The body is the raw file, with no
    /// Content-Type, as the crawler sends it. Its bounds are a transfer's (`transfer_timeouts`), not the 30 s of
    /// the whole search request: a large photo on a slow link takes longer than that.
    pub fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError> {
        self.pace();
        let req = self.agent.post(UPLOAD_URL).header("User-Agent", USER_AGENT).header("Cookie", &self.yp);
        parse_upload(&read_response(transfer_timeouts(req).send(image))?)
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
    /// Only a failure on the first page is an error. Yandex answers page 1 with a captcha (2026-10-31, the crawler
    /// too), so a failure on a later page ends the list with the pages already read, and `stopped` says why.
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
        let req = req.header("User-Agent", USER_AGENT).header("Cookie", &self.yp);
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

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

/// `req` with a transfer's bounds in place of the agent's whole-request one (`TIMEOUT`, which the search requests
/// keep): `STEP_TIMEOUT` for the name lookup, the connection, the request's headers, the wait for the answer's headers
/// and each read or write (a stalled transfer), and `BODY_TIMEOUT` for sending or receiving a body as a whole (one that
/// trickles). The agent's own configuration is not changed.
fn transfer_timeouts<B>(req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    req.config()
        .timeout_global(None)
        .timeout_per_call(None)
        .timeout_resolve(Some(STEP_TIMEOUT))
        .timeout_connect(Some(STEP_TIMEOUT))
        .timeout_send_request(Some(STEP_TIMEOUT))
        .timeout_recv_response(Some(STEP_TIMEOUT))
        .timeout_per_read(Some(STEP_TIMEOUT))
        .timeout_per_write(Some(STEP_TIMEOUT))
        .timeout_send_body(Some(BODY_TIMEOUT))
        .timeout_recv_body(Some(BODY_TIMEOUT))
        .build()
}

/// Fetches result images from the hosts they are on (Yandex's CDN, or the site that has the picture). Unlike the
/// search, no search cookie is sent and nothing is paced, so it can be shared by threads that fetch at once.
pub struct Fetcher {
    agent: ureq::Agent,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        Self { agent: agent() }
    }

    /// Fetches a result image. A response whose type says it is not an image (an error page, a video) is an error
    /// before its body is read ([`plausibly_image`]); the bytes of one that may be decide what it is ([`sniffed`]).
    /// Its bounds are a transfer's (`transfer_timeouts`): a large file on a slow link takes minutes.
    pub fn download(&self, url: &str) -> Result<Download, YandexError> {
        let resp = transfer_timeouts(self.agent.get(url).header("User-Agent", USER_AGENT))
            .call()
            .map_err(|e| YandexError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(YandexError::Status(resp.status().as_u16()));
        }
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !plausibly_image(content_type) {
            return Err(YandexError::Unexpected("the result is not an image"));
        }
        let mut bytes = Vec::new();
        resp.into_body()
            .into_reader()
            .take(MAX_DOWNLOAD_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| YandexError::Transport(e.to_string()))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DOWNLOAD_BYTES {
            return Err(YandexError::Unexpected("the image is larger than the download limit"));
        }
        let format = sniffed(&bytes)?;
        Ok(Download { bytes, format })
    }
}

/// Whether a response's Content-Type lets it be one of the images the search returns, so that its body is worth
/// reading: a type of those kinds, under the names real servers send (`image/jpg`, `image/pjpeg`, `image/x-png` among
/// them), or no type that says anything (none, `application/octet-stream`, `binary/octet-stream`). Anything else (a
/// page, a video, a PDF, an SVG) is refused before the body is read. The type only rules out: the bytes decide what the
/// file is ([`sniffed`]), whatever the type or the URL say.
pub fn plausibly_image(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "image/jpeg"
            | "image/jpg"
            | "image/pjpeg"
            | "image/png"
            | "image/x-png"
            | "image/apng"
            | "image/webp"
            | "image/gif"
            | "image/bmp"
            | "image/x-bmp"
            | "image/x-ms-bmp"
            | ""
            | "application/octet-stream"
            | "binary/octet-stream"
    )
}

/// The format of a downloaded file, from its bytes: one of the kinds the search returns. Anything else is not an image.
pub fn sniffed(bytes: &[u8]) -> Result<Format, YandexError> {
    match Format::sniff(bytes) {
        Some(format @ (Format::Jpeg | Format::Png | Format::Webp | Format::Gif | Format::Bmp)) => Ok(format),
        _ => Err(YandexError::Unexpected("the result is not an image")),
    }
}

/// Reads a response body. A captcha page is reported as `Captcha` whatever its status. A body that can't be read is
/// an error only when the status is a success; for an error status the status is what gets reported.
fn read_response(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<String, YandexError> {
    let mut resp = result.map_err(|e| YandexError::Transport(e.to_string()))?;
    let status = resp.status();
    let body = match resp.body_mut().read_to_string() {
        Ok(body) => body,
        Err(e) if status.is_success() => return Err(YandexError::Transport(e.to_string())),
        Err(_) => String::new(),
    };
    if is_captcha_page(&body) {
        return Err(YandexError::Captcha);
    }
    if !status.is_success() {
        return Err(YandexError::Status(status.as_u16()));
    }
    Ok(body)
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

/// The results page's `ImagesApp-*` element carries its state as an HTML-escaped `data-state` attribute.
#[expect(clippy::string_slice, reason = "every index is a find result or the end of an ASCII pattern: a char boundary")]
fn extract_data_state(html: &str) -> Option<&str> {
    let marker = html.find("id=\"ImagesApp-")?;
    let tag_start = html[..marker].rfind("<div")?;
    let tag_end = tag_start + html[tag_start..].find('>')?;
    let tag = &html[tag_start..tag_end];
    let attr = tag.find("data-state=\"")? + "data-state=\"".len();
    let end = tag[attr..].find('"')?;
    Some(&tag[attr..attr + end])
}

/// Decodes the HTML entities that appear in the state attribute. One pass, so `&amp;quot;` stays `&quot;`.
#[expect(
    clippy::string_slice,
    reason = "every index is a find result, or one past an ASCII '&' or ';': a char boundary"
)]
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded =
            tail.find(';').filter(|&end| end <= 10).and_then(|end| decode_entity(&tail[1..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "quot" => Some('"'),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "apos" => Some('\''),
        _ => {
            let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                name.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(code)
        }
    }
}

/// Every group in `cbirOtherSizes`, largest first. Items without a readable size or URL are skipped.
pub fn parse_other_sizes(html: &str) -> Result<Vec<OtherSize>, YandexError> {
    let raw = extract_data_state(html).ok_or(YandexError::Unexpected("results page has no ImagesApp state"))?;
    let state: Value = serde_json::from_str(&decode_entities(raw))
        .map_err(|_| YandexError::Unexpected("results state is not JSON"))?;
    let Some(groups) = state.pointer("/initialState/cbirOtherSizes/items").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (group, list) in groups {
        for item in list.as_array().into_iter().flatten() {
            let (Some(text), Some(url)) = (item["text"].as_str(), item["url"].as_str()) else {
                continue;
            };
            let Some((width, height)) = parse_size(text) else {
                continue;
            };
            out.push(OtherSize { group: group.clone(), width, height, url: url.to_string() });
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(u64::from(s.width) * u64::from(s.height)));
    Ok(out)
}

/// `"714×320"` → (714, 320).
fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('×')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// One page of the similar endpoint. Errors with `Unexpected` when the JSON has no result list.
pub fn parse_similar_page(body: &str) -> Result<SimilarPage, YandexError> {
    let v: Value = serde_json::from_str(body).map_err(|_| YandexError::Unexpected("similar page is not JSON"))?;
    let serp = v
        .pointer("/blocks/1/params/adapterData/serpList")
        .ok_or(YandexError::Unexpected("similar page has no result list"))?;
    let mut items: Vec<SimilarItem> = serp
        .pointer("/items/entities")
        .and_then(Value::as_object)
        .map(|entities| entities.values().filter_map(similar_item).collect())
        .unwrap_or_default();
    items.sort_by_key(|i| i.pos);
    let has_next = serp.pointer("/navi/next").is_some_and(|n| n.is_string());
    Ok(SimilarPage { items, has_next })
}

fn similar_item(entity: &Value) -> Option<SimilarItem> {
    let pos = u32::try_from(entity["pos"].as_u64()?).ok()?;
    let image_url = query_param(entity["url"].as_str()?, "img_url")?;
    let width = entity["origWidth"].as_u64().or_else(|| entity["width"].as_u64()).unwrap_or(0);
    let height = entity["origHeight"].as_u64().or_else(|| entity["height"].as_u64()).unwrap_or(0);
    Some(SimilarItem {
        pos,
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        image_url,
    })
}

/// The decoded value of `key` in a URL's query string.
fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let mut rest = s.as_bytes();
    let mut out = Vec::with_capacity(rest.len());
    while let [b, tail @ ..] = rest {
        if *b == b'%'
            && let [hi, lo, after @ ..] = tail
            && let Some(v) = std::str::from_utf8(&[*hi, *lo]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            rest = after;
            continue;
        }
        out.push(*b);
        rest = tail;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A results page with the state attribute as Yandex writes it: JSON, HTML-escaped inside the attribute.
    fn results_html(items: &str) -> String {
        let state = format!(r#"{{"initialState":{{"cbirOtherSizes":{{"items":{{"small_dups":{items}}}}}}}}}"#);
        let escaped = state.replace('&', "&amp;").replace('"', "&quot;");
        format!(r#"<html><body><div class="Root" id="ImagesApp-Q1" data-state="{escaped}"></div></body></html>"#)
    }

    #[test]
    fn other_sizes_are_read_largest_first_and_entities_decoded() {
        let html = results_html(
            r#"[{"text":"714×320","url":"https://i.example/a.jpg?w=714&h=320"},{"text":"2560×1150","url":"https://i.example/b.jpg"},{"text":"bad","url":"https://i.example/c.jpg"}]"#,
        );
        let sizes = parse_other_sizes(&html).unwrap();
        assert_eq!(sizes.len(), 2, "the item with an unreadable size is skipped");
        assert_eq!((sizes[0].width, sizes[0].height), (2560, 1150));
        assert_eq!(sizes[1].url, "https://i.example/a.jpg?w=714&h=320");
        assert_eq!(sizes[1].group, "small_dups");
    }

    #[test]
    fn a_page_without_other_sizes_gives_an_empty_list() {
        let html = r#"<div id="ImagesApp-X" data-state="{&quot;initialState&quot;:{}}"></div>"#;
        assert!(parse_other_sizes(html).unwrap().is_empty());
    }

    #[test]
    fn a_page_without_state_is_an_error() {
        assert!(matches!(parse_other_sizes("<html></html>"), Err(YandexError::Unexpected(_))));
    }

    #[test]
    fn similar_page_items_come_from_the_img_url_parameter() {
        let body = r#"{"blocks":[{},{"params":{"adapterData":{"serpList":{
            "items":{"entities":{
                "a":{"pos":1,"image":"//thumbs.example/one.jpg","url":"/images/search?cbir_id=x&img_url=https%3A%2F%2Fa.example%2Fone.jpg&pos=1","origWidth":640,"origHeight":480},
                "b":{"pos":0,"url":"/images/search?img_url=https%3A%2F%2Fb.example%2Ftwo.png%3Fq%3D1","width":300,"height":200}
            }},
            "navi":{"next":"/images/search?p=1"}}}}}]}"#;
        let page = parse_similar_page(body).unwrap();
        assert!(page.has_next);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].pos, 0, "sorted by position");
        assert_eq!(page.items[0].image_url, "https://b.example/two.png?q=1");
        assert_eq!((page.items[0].width, page.items[0].height), (300, 200));
        assert_eq!(page.items[1].image_url, "https://a.example/one.jpg");
    }

    #[test]
    fn similar_page_without_next_link_is_the_last_one() {
        let body = r#"{"blocks":[{},{"params":{"adapterData":{"serpList":{"items":{"entities":{}},"navi":{}}}}}]}"#;
        let page = parse_similar_page(body).unwrap();
        assert!(!page.has_next);
        assert!(page.items.is_empty());
    }

    #[test]
    fn similar_page_without_result_list_is_unexpected() {
        assert!(matches!(parse_similar_page(r#"{"error":"x"}"#), Err(YandexError::Unexpected(_))));
    }

    #[test]
    fn entity_decoding_keeps_double_escapes_intact() {
        assert_eq!(decode_entities("&amp;quot;"), "&quot;");
        assert_eq!(decode_entities("a &b c &#x27;d&#39;"), "a &b c 'd'");
        assert_eq!(decode_entities("&unknown; ok"), "&unknown; ok");
    }

    #[test]
    fn data_state_is_found_on_the_element_that_carries_it() {
        let html = r#"<div class="x"></div><div id="ImagesApp-A" data-state="one"></div><div data-state="two"></div>"#;
        assert_eq!(extract_data_state(html), Some("one"));
    }

    #[test]
    fn size_text_parses_with_the_multiplication_sign() {
        assert_eq!(parse_size("714×320"), Some((714, 320)));
        assert_eq!(parse_size("714x320"), None);
    }

    #[test]
    fn text_parameter_is_host_and_path_without_query() {
        assert_eq!(
            host_and_path("https://avatars.mds.yandex.net/get-images-cbir/1/2/orig"),
            "avatars.mds.yandex.net/get-images-cbir/1/2/orig"
        );
        assert_eq!(host_and_path("https://a.example/x/orig?x=1#f"), "a.example/x/orig");
    }

    #[test]
    fn a_later_page_that_fails_stops_the_list_with_its_reason() {
        let item = |pos: u32| SimilarItem { pos, width: 1, height: 1, image_url: format!("u{pos}") };
        let page = |n: u32, has_next: bool| -> Result<SimilarPage, YandexError> {
            Ok(SimilarPage { items: vec![item(n)], has_next })
        };
        // Pages 0 and 1 are read; page 2 is a captcha: the list keeps what it has, and says why it stopped.
        let got = read_similar_pages(5, |p| match p {
            0 | 1 => page(p, true),
            _ => Err(YandexError::Captcha),
        })
        .unwrap();
        assert_eq!(got.items.iter().map(|i| i.pos).collect::<Vec<_>>(), [0, 1]);
        assert!(matches!(got.stopped, Some(YandexError::Captcha)));
        // A failure on the first page is the error itself.
        assert!(matches!(read_similar_pages(5, |_| Err(YandexError::Captcha)), Err(YandexError::Captcha)));
        // A list that ends by itself (no next page) has no reason.
        let got = read_similar_pages(5, |p| page(p, p < 1)).unwrap();
        assert_eq!(got.items.len(), 2);
        assert!(got.stopped.is_none());
    }

    #[test]
    fn similar_request_is_the_json_of_the_crawlers_monorepos_search_py() {
        // The same bytes `json.dumps(request_json, separators=(",", ":"))` gives in `yandex-reverse/search.py` of the
        // crawlers monorepo, which is outside this repository.
        let v: serde_json::Value = serde_json::from_str(SIMILAR_REQUEST).unwrap();
        assert_eq!(v["blocks"][1]["params"]["ajaxKey"], "serpList/fetch");
        assert!(SIMILAR_REQUEST.starts_with(r#"{"blocks":[{"block":"extra-content""#));
    }

    #[test]
    fn the_types_real_servers_send_for_images_are_read() {
        for content_type in ["image/jpeg; charset=x", "image/webp", "IMAGE/PNG", "image/gif", "image/bmp"] {
            assert!(plausibly_image(content_type), "{content_type}");
        }
        // The four that real servers send for real images, which were refused before their bytes were looked at.
        for content_type in ["image/jpg", "image/pjpeg", "image/x-png", "binary/octet-stream"] {
            assert!(plausibly_image(content_type), "{content_type}");
        }
        // No type that says anything: the bytes decide.
        assert!(plausibly_image(""));
        assert!(plausibly_image("application/octet-stream"));
    }

    #[test]
    fn a_type_that_is_not_one_of_the_searchs_images_is_refused_before_the_body() {
        // An error page or an API answer must not be saved as a picture, whatever its URL ends in.
        for content_type in ["text/html; charset=utf-8", "application/json", "text/plain"] {
            assert!(!plausibly_image(content_type), "{content_type}");
        }
        // Nor downloaded up to the cap only to be refused by its bytes.
        for content_type in ["video/mp4", "application/pdf", "image/svg+xml", "image/avif", "image/tiff"] {
            assert!(!plausibly_image(content_type), "{content_type}");
        }
    }

    #[test]
    fn percent_decoding_handles_truncated_escapes() {
        assert_eq!(percent_decode("a%2Fb%3"), "a/b%3");
        assert_eq!(percent_decode("%E2%9C%93"), "✓");
    }

    #[test]
    fn the_bytes_decide_the_format() {
        // A PNG sent as a JPEG is a PNG; a page or a TIFF is not a kind the search returns.
        assert_eq!(sniffed(b"\x89PNG\r\n\x1a\n....").unwrap(), Format::Png);
        assert!(sniffed(b"<html>").is_err());
        assert!(sniffed(b"II*\0\x08\0\0\0").is_err());
    }
}
