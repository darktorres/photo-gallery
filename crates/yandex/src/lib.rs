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
//! (`http`), the search (`client`) and the result files (`fetch`); their public items are re-exported here.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

mod client;
mod fetch;
mod http;
mod parse;
mod types;

pub use client::{Client, DEFAULT_MAX_SIMILAR_PAGES};
pub use fetch::{Fetcher, MAX_DOWNLOAD_BYTES, plausibly_image};
pub use parse::{is_captcha_page, parse_other_sizes, parse_similar_page, parse_upload, read_similar_pages};
pub use types::{Download, OtherSize, Similar, SimilarItem, SimilarPage, Upload, YandexError};
