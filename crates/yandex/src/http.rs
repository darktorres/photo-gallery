//! The HTTP side every request shares: the agent (its TLS and its timeouts), a transfer's bounds, the
//! User-Agent, and a response read into a body or an error.

use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::parse::is_captcha_page;
use crate::types::YandexError;

/// The same User-Agent as the crawlers monorepo's `yandex-reverse/search.py` (outside this repository), which Yandex
/// serves without a captcha.
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36";

/// The whole of a search request (the results page, a similar page): small answers, so one bound covers it all.
const TIMEOUT: Duration = Duration::from_secs(30);

/// An upload or a download moves a file of up to `MAX_DOWNLOAD_BYTES`, which a slow link takes minutes over, so its
/// bounds are per phase instead (`transfer_timeouts`): the name lookup and the connection, the wait for the answer's
/// headers, and each read or write, which bounds a stalled transfer...
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// ...and the body as a whole, which bounds one that trickles: 100 MB in 15 minutes is about 110 kB/s.
const BODY_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The HTTP agent of the search and of the result files: statuses are handed back to the caller (a captcha page's
/// body says what it is), and TLS is the system's: native-tls on the OS's roots (the platform verifier). The default
/// roots (`WebPki`) are the bundled Mozilla list with the system's store off, and a chain the system builds for a site
/// (i.ytimg.com's) then fails: "unable to find any user-specified roots in the final cert chain".
pub(crate) fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(
            TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
        )
        .build();
    ureq::Agent::new_with_config(config)
}

/// `req` with a transfer's bounds in place of the agent's whole-request one (`TIMEOUT`, which the search requests
/// keep): `STEP_TIMEOUT` for the name lookup, the connection, the request's headers, the wait for the answer's headers
/// and each read or write (a stalled transfer), and `BODY_TIMEOUT` for sending or receiving a body as a whole (one that
/// trickles). The agent's own configuration is not changed.
pub(crate) fn transfer_timeouts<B>(req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
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

/// Reads a response body. A captcha page is reported as `Captcha` whatever its status. A body that can't be read is
/// an error only when the status is a success; for an error status the status is what gets reported.
pub(crate) fn read_response(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<String, YandexError> {
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
