//! Yandex Images reverse search: upload a photo, then read the two result lists.
//!
//! - **Other sizes:** the same image at other resolutions. It sits in the results page's initial state
//!   (`cbirOtherSizes`), so one request gets all of them.
//! - **Similar:** visually similar images, from the `cbir_page=similar` JSON endpoint, paged with `p`.
//!
//! The client keeps the cookies Yandex sets, as the crawler's `requests.Session` does, so each request carries the ones
//! the responses before it gave. It starts with the `yp` cookie of screen metrics (`<ts>.szm.1:WxH:WxH`), which is what
//! Yandex's own script writes into a browser. Nothing else is sent: the Sites list is not fetched, because its pages
//! after the first need an anti-robot cookie, and this client does not try to get one.
//!
//! Endpoints and parsing follow `yandex-reverse/search.py` of the crawlers monorepo, which is outside this repository.
//!
//! The modules are the results and errors (`types`), the answers read (`parse`), the HTTP side every request shares
//! (`http`), the search (`client`), the result files (`fetch`), the addresses of the files (`result_url`) and the
//! per-server limit on them (`gate`); their public items are re-exported here.

#![forbid(unsafe_code)]

mod client;
mod fetch;
mod gate;
mod http;
mod parse;
mod result_url;
mod types;

pub use client::{Client, DEFAULT_MAX_SIMILAR_PAGES};
pub use fetch::{Fetcher, MAX_DOWNLOAD_BYTES, MAX_REDIRECTS, plausibly_image};
pub use gate::{Cancelled, Gate, Kick, PER_SERVER, Permit};
pub use parse::{is_captcha_page, parse_other_sizes, parse_similar_page, parse_upload, read_similar_pages};
pub use result_url::{ResultUrl, Server};
pub use types::{Download, OtherSize, Similar, SimilarItem, SimilarPage, Upload, YandexError};
