//! The result files: fetched from the hosts they are on, refused by their type before their body, and known by
//! their bytes.

use std::io::Read;

use pg_model::format::Format;

use crate::http::{USER_AGENT, agent, transfer_timeouts};
use crate::types::{Download, YandexError};

/// Largest result image `Fetcher::download` accepts.
pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Fetches result images from the hosts they are on (Yandex's CDN, or the site that has the picture). Unlike the
/// search, nothing is paced, so it can be shared by threads that fetch at once. Its jar holds only what those hosts set:
/// the search's cookies are for Yandex's origin, and no other host is sent them.
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
    /// before its body is read ([`plausibly_image`]); the bytes of one that may be decide what it is (`Format::sniff`).
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
        let format = Format::sniff(&bytes);
        Ok(Download { bytes, format })
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
    use super::*;

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
}
