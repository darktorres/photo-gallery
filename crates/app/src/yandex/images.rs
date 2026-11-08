//! The pictures of a search's results. Each result's file is fetched once, from the host it is on, and kept for as long as
//! the search is shown: the grid shows it, and a save writes the same bytes. A file that can't be fetched, or that isn't
//! an image this app decodes, is dead: the result is taken off the grid (`Poll::dead`), and it is never saved. The
//! files are fetched as the results come in, a few at a time, so dead links are known before they are shown.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread;

use pg_jobs::events::Bus;
use pg_model::format::Format;
use pg_model::turns::Turns;
use pg_render::maker::Maker;
use pg_yandex::{Download, Fetcher, YandexError};

/// The height of a grid picture, in px. The gallery's thumbnail pipeline makes it (a DCT-scaled decode, Lanczos3 down,
/// a JPEG), so a result looks like a gallery thumbnail; the grid draws at 220 px.
pub const GRID_HEIGHT: usize = 320;

/// How many files are fetched at once.
const FETCHERS: usize = 4;

/// Fetches a result's file from the host it is on.
pub trait Fetch: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Download, YandexError>;
}

impl Fetch for Fetcher {
    fn fetch(&self, url: &str) -> Result<Download, YandexError> {
        self.download(url)
    }
}

/// Makes the grid picture of bytes that are an image this app decodes; the reason when they are not.
/// [`grid_picture`] is the real one.
pub type Check = fn(&[u8]) -> Result<Vec<u8>, &'static str>;

/// A result's file, kept: the file as Yandex's host gave it (what a save writes), and its grid picture (a JPEG,
/// [`GRID_HEIGHT`] px high, which the grid shows).
#[derive(Debug)]
pub struct Fetched {
    pub file: Download,
    pub grid: Vec<u8>,
}

/// Where a result's file is.
enum State {
    /// Waiting for a fetcher.
    Queued,
    /// Being fetched now, by a fetcher or by a caller that asked for it.
    Fetching,
    Ready(Arc<Fetched>),
    Dead(YandexError),
}

struct Slot {
    url: String,
    state: State,
}

#[derive(Default)]
struct Table {
    slots: BTreeMap<u64, Slot>,
    /// The results waiting for a fetcher, in order.
    queue: VecDeque<u64>,
    /// The dead results, until `take_dead` reports them.
    dead: Vec<u64>,
}

/// The files of one search's results, by result id.
pub struct Images {
    table: Mutex<Table>,
    /// Signalled when a file leaves `Fetching`.
    settled: Condvar,
    fetch: Arc<dyn Fetch>,
    check: Check,
    changes: Arc<Bus<()>>,
}

fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Images {
    /// The files of a search, fetched with `fetch` and checked with `check`. Each dead result is announced on `changes`.
    pub fn new(fetch: Arc<dyn Fetch>, check: Check, changes: Arc<Bus<()>>) -> Arc<Images> {
        Arc::new(Images { table: Mutex::default(), settled: Condvar::new(), fetch, check, changes })
    }

    /// Queues the results `(id, url)` for fetching, and starts the fetchers for them. A result that no fetcher reaches
    /// is fetched when it is asked for ([`Images::get`]).
    pub fn prefetch(self: &Arc<Self>, results: Vec<(u64, String)>) {
        let count = results.len();
        {
            let mut t = lock(&self.table);
            for (id, url) in results {
                t.slots.insert(id, Slot { url, state: State::Queued });
                t.queue.push_back(id);
            }
        }
        for n in 0..count.min(FETCHERS) {
            let images = Arc::clone(self);
            // A thread that can't start leaves its results queued: `get` fetches them.
            let _ = thread::Builder::new().name(format!("yandex image {n}")).spawn(move || images.fetch_queue());
        }
    }

    /// A fetcher: takes the queued results in order, until none is left.
    fn fetch_queue(&self) {
        while let Some((id, url)) = self.next_queued() {
            let outcome = self.fetch_caught(&url);
            self.settle(id, outcome);
        }
    }

    /// The next result still waiting, marked as being fetched. None when none is waiting.
    fn next_queued(&self) -> Option<(u64, String)> {
        let mut t = lock(&self.table);
        while let Some(id) = t.queue.pop_front() {
            if let Some(slot) = t.slots.get_mut(&id)
                && matches!(slot.state, State::Queued)
            {
                slot.state = State::Fetching;
                return Some((id, slot.url.clone()));
            }
        }
        None
    }

    fn fetch_checked(&self, url: &str) -> Result<Arc<Fetched>, YandexError> {
        let file = self.fetch.fetch(url)?;
        let grid = (self.check)(&file.bytes).map_err(YandexError::Undecodable)?;
        Ok(Arc::new(Fetched { file, grid }))
    }

    /// [`Images::fetch_checked`], where a panic (the decoder's, on a malformed file) ends the result as dead: a result
    /// left in `Fetching` would hold up every caller that waits for it.
    fn fetch_caught(&self, url: &str) -> Result<Arc<Fetched>, YandexError> {
        match pg_jobs::panics::catch(AssertUnwindSafe(|| self.fetch_checked(url))) {
            Ok(outcome) => outcome,
            Err(_) => Err(YandexError::Undecodable("the decoder stopped on this file")),
        }
    }

    /// Keeps the outcome of a fetch: its file, or the death of the result (announced on `changes`).
    fn settle(&self, id: u64, outcome: Result<Arc<Fetched>, YandexError>) {
        let dead = outcome.is_err();
        {
            let mut t = lock(&self.table);
            let state = match outcome {
                Ok(download) => State::Ready(download),
                Err(error) => {
                    t.dead.push(id);
                    State::Dead(error)
                }
            };
            if let Some(slot) = t.slots.get_mut(&id) {
                slot.state = state;
            }
        }
        self.settled.notify_all();
        if dead {
            self.changes.publish(());
        }
    }

    /// The file of the result `id`, which is at `url`: kept when it is in, waited for when a fetcher is on it, and
    /// fetched now otherwise. A dead result is an error.
    pub fn get(&self, id: u64, url: &str) -> Result<Arc<Fetched>, YandexError> {
        let mut t = lock(&self.table);
        let url = loop {
            match t.slots.get(&id) {
                Some(Slot { state: State::Ready(fetched), .. }) => return Ok(fetched.clone()),
                Some(Slot { state: State::Dead(error), .. }) => return Err(error.clone()),
                Some(Slot { state: State::Fetching, .. }) => {}
                Some(Slot { url, state: State::Queued }) => break url.clone(),
                None => break url.to_string(),
            }
            t = self.settled.wait(t).unwrap_or_else(PoisonError::into_inner);
        };
        t.slots.insert(id, Slot { url: url.clone(), state: State::Fetching });
        drop(t);
        let outcome = self.fetch_caught(&url);
        self.settle(id, outcome.clone());
        outcome
    }

    /// The file of the result `id`, at the URL it was queued with. An id that no search has is an error.
    pub fn get_known(&self, id: u64) -> Result<Arc<Fetched>, YandexError> {
        let url = lock(&self.table)
            .slots
            .get(&id)
            .map(|slot| slot.url.clone())
            .ok_or(YandexError::Unexpected("no such result"))?;
        self.get(id, &url)
    }

    /// The results that died since the last call, each once.
    pub fn take_dead(&self) -> Vec<u64> {
        std::mem::take(&mut lock(&self.table).dead)
    }
}

/// The images of the search on screen: what the grid's picture requests and a save read. A new search replaces them.
static CURRENT: RwLock<Option<Arc<Images>>> = RwLock::new(None);

/// Makes `images` the search's, from now on.
pub fn set_current(images: Option<Arc<Images>>) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = images;
}

/// The images of the search on screen, if there is one.
pub fn current() -> Option<Arc<Images>> {
    CURRENT.read().unwrap_or_else(PoisonError::into_inner).clone()
}

thread_local! {
    /// This thread's maker: its decoder, resizer and encoder keep their buffers, so each fetcher has one.
    static MAKER: RefCell<Option<Maker>> = const { RefCell::new(None) };
}

/// The grid picture of `bytes`, an image this app decodes: a JPEG [`GRID_HEIGHT`] px high, made by the gallery's
/// thumbnail pipeline. The reason when the bytes are not such an image.
pub fn grid_picture(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if Format::sniff(bytes).is_none() {
        return Err("the file is not a picture");
    }
    MAKER.with_borrow_mut(|slot| -> Result<Vec<u8>, &'static str> {
        if slot.is_none() {
            *slot = Some(Maker::new().map_err(|_| "no maker")?);
        }
        let Some(maker) = slot.as_mut() else {
            return Err("no maker");
        };
        maker.make(bytes, GRID_HEIGHT, Turns::NONE).map_err(|_| "the picture does not decode")
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use pg_model::format::Format;

    use super::*;

    /// Serves `body` for every URL except those in `fail`, and counts the fetches.
    struct Counting {
        calls: AtomicUsize,
        fail: Vec<&'static str>,
    }

    impl Fetch for Counting {
        fn fetch(&self, url: &str) -> Result<Download, YandexError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.contains(&url) {
                return Err(YandexError::Status(404));
            }
            Ok(Download { bytes: b"picture".to_vec(), format: Format::Jpeg })
        }
    }

    fn accept(_: &[u8]) -> Result<Vec<u8>, &'static str> {
        Ok(b"grid".to_vec())
    }

    fn reject(_: &[u8]) -> Result<Vec<u8>, &'static str> {
        Err("no")
    }

    /// Bytes that are no picture make no grid picture.
    #[test]
    fn a_file_that_is_no_picture_makes_no_grid_picture() {
        assert_eq!(grid_picture(b"not a picture at all"), Err("the file is not a picture"));
    }

    fn images(fetch: Arc<Counting>, check: Check) -> Arc<Images> {
        Images::new(fetch, check, Arc::new(Bus::new(16)))
    }

    /// A result fetched in the background is kept: asking for it again fetches nothing.
    #[test]
    fn a_result_is_fetched_once_and_kept() {
        let fetch = Arc::new(Counting { calls: AtomicUsize::new(0), fail: vec![] });
        let images = images(fetch.clone(), accept);
        images.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        let first = images.get(1, "https://a.example/1.jpg").unwrap();
        let again = images.get(1, "https://a.example/1.jpg").unwrap();
        assert_eq!(first.file.bytes, b"picture");
        assert_eq!(first.grid, b"grid");
        assert!(Arc::ptr_eq(&first, &again));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 1, "one fetch for the file");
    }

    /// A file that can't be fetched is dead: asking for it fails, and the death is reported once.
    #[test]
    fn a_result_that_cannot_be_fetched_is_dead_and_reported_once() {
        let fetch = Arc::new(Counting { calls: AtomicUsize::new(0), fail: vec!["https://gone.example/2.jpg"] });
        let images = images(fetch, accept);
        images.prefetch(vec![(2, "https://gone.example/2.jpg".into())]);
        assert!(images.get(2, "https://gone.example/2.jpg").is_err());
        assert_eq!(images.take_dead(), vec![2]);
        assert!(images.take_dead().is_empty(), "reported once");
    }

    /// A file that is not an image this app decodes is dead, with the check's reason.
    #[test]
    fn a_file_that_does_not_decode_is_dead() {
        let fetch = Arc::new(Counting { calls: AtomicUsize::new(0), fail: vec![] });
        let images = images(fetch, reject);
        let error = images.get(3, "https://a.example/3.png").unwrap_err();
        assert!(matches!(error, YandexError::Undecodable("no")), "{error:?}");
        assert_eq!(images.take_dead(), vec![3]);
    }

    /// A result with no fetcher left is fetched when it is asked for, and by its URL.
    #[test]
    fn a_result_asked_for_before_it_is_queued_is_fetched_by_its_url() {
        let fetch = Arc::new(Counting { calls: AtomicUsize::new(0), fail: vec![] });
        let images = images(fetch.clone(), accept);
        assert_eq!(images.get_known(9).unwrap_err().to_string(), "unexpected Yandex response: no such result");
        assert_eq!(images.get(9, "https://a.example/9.jpg").unwrap().file.bytes, b"picture");
        assert_eq!(images.get_known(9).unwrap().file.bytes, b"picture");
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 1);
    }
}
