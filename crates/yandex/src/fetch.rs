//! The result files: fetched from the servers they are on, refused by their type before their body, and known by their
//! bytes. Each request is in flight under a slot of its server (`gate`), and a redirect is followed by hand, so every hop
//! takes the slot of the server it goes to.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pg_model::format::Format;

use crate::gate::{Cancelled, Gate, PER_SERVER, Permit};
use crate::http::{USER_AGENT, agent_config, transfer_timeouts};
use crate::result_url::ResultUrl;
use crate::types::{Download, YandexError};

/// Largest result image `Fetcher::download` accepts.
pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Redirects one download follows: the requests it makes are this many plus one. Past it, the download fails.
pub const MAX_REDIRECTS: u32 = 5;

/// The most of a redirect's body that is read, so the connection can go back to the pool. A longer one is dropped with
/// its connection.
const DRAIN_LIMIT: u64 = 16 * 1024;

/// Idle connections kept for the result files: one per request a server may have in flight, and a total for several
/// servers at once.
const IDLE_TOTAL: usize = 4 * PER_SERVER;

/// Fetches result images from the servers they are on (Yandex's CDN, or the site that has the picture). Unlike the
/// search, nothing is paced, so it can be shared by threads that fetch at once. Its jar holds only what those servers
/// set: the search's cookies are for Yandex's origin, and no other server is sent them.
pub struct Fetcher {
    agent: ureq::Agent,
    gate: Arc<Gate>,
}

impl Fetcher {
    /// A fetcher with an HTTP agent of its own, whose requests are counted by `gate`.
    pub fn new(gate: Arc<Gate>) -> Self {
        let config = agent_config().max_idle_connections_per_host(PER_SERVER).max_idle_connections(IDLE_TOTAL).build();
        Self { agent: ureq::Agent::new_with_config(config), gate }
    }

    /// The gate that counts this fetcher's requests.
    pub fn gate(&self) -> &Arc<Gate> {
        &self.gate
    }

    /// Fetches a result image. `first` is the slot of the file's own server, which the caller took (see [`Gate`]); a
    /// redirect to another server takes that server's slot before its request. A response whose type says it is not an
    /// image (an error page, a video) is an error before its body is read ([`plausibly_image`]); the bytes of one that may
    /// be decide what it is (`Format::sniff`). The slot of the file's server is given back once its body is read, so a
    /// decode does not hold it. `cancel` is checked before each request and each read, so a download is cut short once it
    /// is set (the app sets it at close); a request already waiting for a server's slot ends with it too.
    pub fn download(&self, url: &ResultUrl, first: Permit, cancel: &AtomicBool) -> Result<Download, YandexError> {
        if !Arc::ptr_eq(first.gate(), &self.gate) || first.server() != url.server() {
            return Err(YandexError::Transport("the slot is not of this file's server".to_string()));
        }
        let mut permit = first;
        let mut current = url.clone();
        let mut redirects = 0;
        let resp = loop {
            let resp = self.request(&current, cancel)?;
            if !is_redirect(resp.status().as_u16()) {
                break resp;
            }
            let status = resp.status().as_u16();
            let Some(location) = last_location(&resp) else {
                return Err(YandexError::Status(status));
            };
            if redirects == MAX_REDIRECTS {
                return Err(YandexError::Transport("too many redirects".to_string()));
            }
            redirects += 1;
            let next = current
                .join(&location)
                .ok_or_else(|| YandexError::Transport("a redirect to an address that is not a web one".to_string()))?;
            // The redirect's response is read on and dropped first: its connection is still open until then.
            drain_small(resp, cancel)?;
            if next.server() != current.server() {
                drop(permit);
                permit = self.gate.acquire(next.server(), cancel).map_err(cut_short)?;
            }
            current = next;
        };
        if !resp.status().is_success() {
            return Err(YandexError::Status(resp.status().as_u16()));
        }
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !plausibly_image(content_type) {
            return Err(YandexError::Unexpected(pg_model::yandex::UnexpectedReason::NotAnImage));
        }
        let mut body = resp.into_body().into_reader().take(MAX_DOWNLOAD_BYTES + 1);
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(cut_short(Cancelled));
            }
            let n = body.read(&mut chunk).map_err(|e| YandexError::Transport(e.to_string()))?;
            if n == 0 {
                break;
            }
            let read =
                chunk.get(..n).ok_or_else(|| YandexError::Transport("a read longer than its buffer".to_string()))?;
            bytes.extend_from_slice(read);
        }
        drop(permit);
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DOWNLOAD_BYTES {
            return Err(YandexError::Unexpected(pg_model::yandex::UnexpectedReason::TooLarge));
        }
        let format = Format::sniff(&bytes);
        Ok(Download { bytes, format })
    }

    /// One request to `url`, answered with the response as it is (a redirect is not followed by the agent): the
    /// transfer's bounds apply to it.
    fn request(&self, url: &ResultUrl, cancel: &AtomicBool) -> Result<ureq::http::Response<ureq::Body>, YandexError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(cut_short(Cancelled));
        }
        transfer_timeouts(self.agent.get(url.as_str()).header("User-Agent", USER_AGENT))
            .config()
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| YandexError::Transport(e.to_string()))
    }
}

/// A download the close cut short: the same error a body read gives.
fn cut_short(_: Cancelled) -> YandexError {
    YandexError::Transport("the download was cut short at close".to_string())
}

/// Whether a response with this status is a redirect this download follows: any 3xx but 304 (not modified), which is the
/// rule ureq itself applies.
fn is_redirect(status: u16) -> bool {
    (300..400).contains(&status) && status != 304
}

/// The target of a redirect: its last `Location` header, which is the one ureq follows.
fn last_location(resp: &ureq::http::Response<ureq::Body>) -> Option<String> {
    resp.headers().get_all("location").iter().next_back().and_then(|value| value.to_str().ok()).map(str::to_string)
}

/// Reads a redirect's body to its end, up to [`DRAIN_LIMIT`], so its connection can be reused, and drops the response.
/// A body longer than that, a read that fails, or a close is not waited for: the connection is simply not reused.
fn drain_small(mut resp: ureq::http::Response<ureq::Body>, cancel: &AtomicBool) -> Result<(), YandexError> {
    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 4096];
    let mut total: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cut_short(Cancelled));
        }
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(n) => {
                total = total.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                if total > DRAIN_LIMIT {
                    return Ok(());
                }
            }
        }
    }
}

/// Whether a response's Content-Type lets its body be read as one of the images: any `image/*` type (the names real
/// servers send, `image/jpg`, `image/pjpeg` and `image/x-png` among them, and types the app may not decode, such as
/// `image/tiff` or `image/avif`), except SVG, which is markup that can run scripts; or no type that says anything
/// (none, `application/octet-stream`, `binary/octet-stream`). Anything else (a page, a video, a PDF, an SVG) is refused
/// before the body is read. The type only rules out: the bytes decide what the file is (`Format::sniff`), whatever the type
/// or the URL say.
pub fn plausibly_image(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match essence.as_str() {
        "" | "application/octet-stream" | "binary/octet-stream" => true,
        _ => essence.strip_prefix("image/").is_some_and(|kind| !kind.starts_with("svg")),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Arc;

    use super::*;

    /// Serves, on a port of its own, whatever `answer` makes of each request's path, one connection after another. The
    /// answer is the whole response, status line and headers included.
    fn serve(answer: impl Fn(&str, u16) -> String + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request).to_string();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let _ = stream.write_all(answer(&path, port).as_bytes());
            }
        });
        port
    }

    fn image(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// A fetcher whose agent has no proxy, so loopback is reached directly.
    fn fetcher(gate: Arc<Gate>) -> Fetcher {
        let config = agent_config().proxy(None).max_idle_connections_per_host(PER_SERVER).build();
        Fetcher { agent: ureq::Agent::new_with_config(config), gate }
    }

    /// A slot for `url`, as a caller takes it before a download.
    fn slot(gate: &Arc<Gate>, url: &ResultUrl) -> Permit {
        gate.try_acquire(url.server()).unwrap()
    }

    #[test]
    fn a_redirect_to_another_server_is_followed_and_the_slots_are_given_back() {
        let far = serve(|_, _| image("the picture"));
        let near = serve(move |_, _| {
            format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{far}/moved.jpg\r\nContent-Length: 0\r\n\r\n")
        });
        let gate = Gate::new(PER_SERVER);
        let url = ResultUrl::parse(&format!("http://127.0.0.1:{near}/a.jpg")).unwrap();
        let download = fetcher(gate.clone()).download(&url, slot(&gate, &url), &AtomicBool::new(false)).unwrap();
        assert_eq!(download.bytes, b"the picture");
        assert_eq!(gate.in_flight(url.server()), 0, "the first server's slot is given back");
        let far_url = ResultUrl::parse(&format!("http://127.0.0.1:{far}/x")).unwrap();
        assert_eq!(gate.in_flight(far_url.server()), 0, "the last server's slot is given back after the body");
    }

    #[test]
    fn a_redirect_chain_longer_than_the_limit_fails() {
        let port = serve(|_, port| {
            format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/again\r\nContent-Length: 0\r\n\r\n")
        });
        let gate = Gate::new(PER_SERVER);
        let url = ResultUrl::parse(&format!("http://127.0.0.1:{port}/start")).unwrap();
        let result = fetcher(gate.clone()).download(&url, slot(&gate, &url), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Transport(ref e)) if e == "too many redirects"), "{result:?}");
        assert_eq!(gate.in_flight(url.server()), 0, "the slot is given back on failure");
    }

    /// A server whose `/hop/k` redirects to `/hop/k+1` while `k` is below `redirects`, and answers the picture after.
    fn hops(redirects: u32) -> u16 {
        serve(move |path, port| {
            let hop: u32 = path.rsplit('/').next().and_then(|n| n.parse().ok()).unwrap_or(0);
            if hop < redirects {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/hop/{}\r\nContent-Length: 0\r\n\r\n",
                    hop + 1
                )
            } else {
                image("the picture")
            }
        })
    }

    /// Exactly `MAX_REDIRECTS` redirects are followed to the picture; one more fails.
    #[test]
    fn exactly_the_limit_of_redirects_is_followed_and_one_more_is_not() {
        let gate = Gate::new(PER_SERVER);
        let within = ResultUrl::parse(&format!("http://127.0.0.1:{}/hop/0", hops(MAX_REDIRECTS))).unwrap();
        let download = fetcher(gate.clone()).download(&within, slot(&gate, &within), &AtomicBool::new(false));
        assert_eq!(download.unwrap().bytes, b"the picture");
        let beyond = ResultUrl::parse(&format!("http://127.0.0.1:{}/hop/0", hops(MAX_REDIRECTS + 1))).unwrap();
        let result = fetcher(gate.clone()).download(&beyond, slot(&gate, &beyond), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Transport(ref e)) if e == "too many redirects"), "{result:?}");
    }

    /// A download that is cut short before its first request makes none, and fails as cut short.
    #[test]
    fn a_download_cut_short_before_its_request_fails_as_cut_short() {
        let gate = Gate::new(PER_SERVER);
        let url = ResultUrl::parse(&format!("http://127.0.0.1:{}/x", hops(0))).unwrap();
        let result = fetcher(gate.clone()).download(&url, slot(&gate, &url), &AtomicBool::new(true));
        assert!(matches!(result, Err(YandexError::Transport(ref e)) if e.contains("cut short")), "{result:?}");
        assert_eq!(gate.in_flight(url.server()), 0, "the slot is given back");
    }

    #[test]
    fn a_not_modified_is_not_followed_and_a_redirect_without_a_location_is_a_status() {
        let port = serve(|path, _| match path {
            "/304" => {
                "HTTP/1.1 304 Not Modified\r\nLocation: http://127.0.0.1:1/x\r\nContent-Length: 0\r\n\r\n".to_string()
            }
            _ => "HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n".to_string(),
        });
        let gate = Gate::new(PER_SERVER);
        let not_modified = ResultUrl::parse(&format!("http://127.0.0.1:{port}/304")).unwrap();
        let result = fetcher(gate.clone()).download(&not_modified, slot(&gate, &not_modified), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Status(304))), "{result:?}");
        let no_location = ResultUrl::parse(&format!("http://127.0.0.1:{port}/302")).unwrap();
        let result = fetcher(gate.clone()).download(&no_location, slot(&gate, &no_location), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Status(302))), "{result:?}");
    }

    #[test]
    fn a_slot_of_another_server_is_refused() {
        let gate = Gate::new(PER_SERVER);
        let url = ResultUrl::parse("http://127.0.0.1:9/x.jpg").unwrap();
        let other = ResultUrl::parse("http://127.0.0.1:10/x.jpg").unwrap();
        let result = fetcher(gate.clone()).download(&url, slot(&gate, &other), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Transport(_))), "{result:?}");
        let elsewhere = Gate::new(PER_SERVER);
        let result =
            fetcher(gate.clone()).download(&url, elsewhere.try_acquire(url.server()).unwrap(), &AtomicBool::new(false));
        assert!(matches!(result, Err(YandexError::Transport(_))), "a slot of another gate: {result:?}");
    }

    #[test]
    fn a_type_that_is_not_an_image_is_refused_before_the_body() {
        // An error page or an API answer must not be saved as a picture, whatever its URL ends in.
        for content_type in ["text/html; charset=utf-8", "application/json", "text/plain"] {
            assert!(!plausibly_image(content_type), "{content_type}");
        }
        // Nor downloaded up to the cap only to be refused by its bytes. SVG is markup, so it is refused as an image too.
        for content_type in ["video/mp4", "application/pdf", "image/svg+xml", "IMAGE/SVG+XML; charset=utf-8"] {
            assert!(!plausibly_image(content_type), "{content_type}");
        }
    }

    #[test]
    fn the_image_types_are_read_whatever_their_name() {
        for content_type in ["image/jpeg; charset=x", "image/webp", "IMAGE/PNG", "image/gif", "image/bmp"] {
            assert!(plausibly_image(content_type), "{content_type}");
        }
        // The names real servers send for real images, which were refused before their bytes were looked at.
        for content_type in ["image/jpg", "image/pjpeg", "image/x-png", "binary/octet-stream"] {
            assert!(plausibly_image(content_type), "{content_type}");
        }
        // Image types the app may not decode are still read: the bytes decide, not the type.
        for content_type in ["image/avif", "image/tiff", "image/heic"] {
            assert!(plausibly_image(content_type), "{content_type}");
        }
        // No type that says anything: the bytes decide.
        assert!(plausibly_image(""));
        assert!(plausibly_image("application/octet-stream"));
    }
}
