//! Every Yandex search response, in full, for the log: one file per response in `yandex-responses`, beside
//! `gallery.log`. A file starts with the endpoint the response answers, then its status, its error (if any), the request
//! as it was sent, the response's headers and its body, as they came. The names start with the time a response came, so the folder reads in order. The files
//! hold what Yandex sent back, the cookies among its headers: they are for reading, not for sharing.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pg_app::yandex::{Dump, Exchange};

/// The responses written so far, to tell apart two that come in the same millisecond.
static COUNT: AtomicU64 = AtomicU64::new(0);

/// The dump the search's client writes to, or `None` when there is no log folder (`panic_log::install` has not run).
pub fn dump() -> Option<Dump> {
    let folder = crate::panic_log::beside_log("yandex-responses")?;
    Some(Arc::new(move |exchange: &Exchange| write(&folder, exchange)))
}

/// Writes one response to `folder`. A file that can't be written is noted in the log, and the search goes on.
fn write(folder: &Path, exchange: &Exchange) {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let name = format!("{millis:013}-{n:06}-{}.txt", slug(&exchange.endpoint));
    let status = exchange.status.map_or_else(|| "none".to_string(), |status| status.to_string());
    let error = exchange.error.as_deref().unwrap_or("none");
    let text = format!(
        "endpoint: {}\nstatus: {status}\nerror: {error}\n\nrequest:\n{}\nresponse headers:\n{}\nbody:\n{}",
        exchange.endpoint, exchange.request, exchange.headers, exchange.body
    );
    let written = std::fs::create_dir_all(folder).and_then(|()| std::fs::write(folder.join(&name), text));
    if let Err(e) = written {
        crate::panic_log::note(&format!("yandex response {name}: {e}"));
    }
}

/// `endpoint` as part of a file name: its letters and digits, and a dash for anything else.
fn slug(endpoint: &str) -> String {
    endpoint.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_is_written_whole_under_its_endpoint() {
        let folder = std::env::temp_dir().join(format!("pg-yandex-responses-{}", std::process::id()));
        let exchange = Exchange {
            endpoint: "similar page 2".to_string(),
            request: "method: GET\n".to_string(),
            status: Some(200),
            headers: "content-type: application/json\n".to_string(),
            body: "{\"blocks\":[]}\n".to_string(),
            error: None,
        };
        write(&folder, &exchange);
        let files: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(files.len(), 1);
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-similar-page-2.txt"), "{name}");
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert_eq!(
            text,
            "endpoint: similar page 2\nstatus: 200\nerror: none\n\nrequest:\nmethod: GET\n\nresponse headers:\ncontent-type: application/json\n\nbody:\n{\"blocks\":[]}\n"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
