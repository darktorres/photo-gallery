//! The pictures of a search's results. Each result's file is fetched once, from the host it is on, and kept for as long as
//! the search is shown: the grid shows it, and a save writes the same bytes. A file that can't be fetched, or that isn't
//! an image this app decodes, is dead: the result is taken off the grid (`Poll::dead`), and it is never saved. The
//! files are fetched as the results come in, a few at a time, so dead links are known before they are shown.

use std::collections::{BTreeMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use pg_jobs::events::Bus;
use pg_model::format::Format;
use pg_model::turns::Turns;
use pg_yandex::{Download, Fetcher, YandexError};

use super::source::SourceError;
use crate::makers::Makers;

/// How many files are fetched at once.
const FETCHERS: usize = 4;

/// Fetches a result's file from the host it is on.
pub trait Fetch: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Download, SourceError>;
}

impl Fetch for Fetcher {
    fn fetch(&self, url: &str) -> Result<Download, SourceError> {
        Ok(self.download(url)?)
    }
}

/// Makes the grid picture of bytes that are an image this app decodes; the reason when they are not.
/// The app's is `grid_picture`, with its makers and the panel's row height.
pub type Check = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, &'static str> + Send + Sync>;

/// A result's file, kept: the file as Yandex's host gave it (what a save writes), its format (a save's extension), and
/// its grid picture (a JPEG as high as the panel's rows, which the grid shows).
#[derive(Debug)]
pub struct Fetched {
    pub file: Download,
    pub format: Format,
    pub grid: Vec<u8>,
}

/// Where a result's file is.
enum State {
    /// Waiting for a fetcher.
    Queued,
    /// Being fetched now, by a fetcher or by a caller that asked for it.
    Fetching,
    Ready(Arc<Fetched>),
    Dead(SourceError),
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
    /// The fetchers running.
    fetchers: usize,
    /// Set by [`Images::stop`]: the fetchers take no more results.
    stopped: bool,
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
            lock(&self.table).fetchers += 1;
            // A thread that can't start leaves its results queued: `get` fetches them.
            if thread::Builder::new().name(format!("yandex image {n}")).spawn(move || images.fetch_queue()).is_err() {
                lock(&self.table).fetchers -= 1;
            }
        }
    }

    /// A fetcher: takes the queued results in order, until none is left or the images are stopped.
    fn fetch_queue(&self) {
        while let Some((id, url)) = self.next_queued() {
            let outcome = self.fetch_caught(&url);
            self.settle(id, outcome);
        }
        lock(&self.table).fetchers -= 1;
        self.settled.notify_all();
    }

    /// The fetchers take no more results; the ones fetching end with the file in hand.
    pub fn stop(&self) {
        lock(&self.table).stopped = true;
    }

    /// No fetcher is running.
    pub fn idle(&self) -> bool {
        lock(&self.table).fetchers == 0
    }

    /// The next result still waiting, marked as being fetched. None when none is waiting, or the images are stopped.
    fn next_queued(&self) -> Option<(u64, String)> {
        let mut t = lock(&self.table);
        if t.stopped {
            return None;
        }
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

    /// The file at `url`, fetched, and kept only when the app decodes it: a format the model knows, which the check makes
    /// a grid picture of.
    fn fetch_checked(&self, url: &str) -> Result<Arc<Fetched>, SourceError> {
        let file = self.fetch.fetch(url)?;
        let format = file.format.ok_or(SourceError::Undecodable("not a format this app knows"))?;
        let grid = (self.check)(&file.bytes).map_err(SourceError::Undecodable)?;
        Ok(Arc::new(Fetched { file, format, grid }))
    }

    /// [`Images::fetch_checked`], where a panic (the decoder's, on a malformed file) ends the result as dead: a result
    /// left in `Fetching` would hold up every caller that waits for it.
    fn fetch_caught(&self, url: &str) -> Result<Arc<Fetched>, SourceError> {
        match pg_jobs::panics::catch(AssertUnwindSafe(|| self.fetch_checked(url))) {
            Ok(outcome) => outcome,
            Err(_) => Err(SourceError::Undecodable("the decoder stopped on this file")),
        }
    }

    /// Keeps the outcome of a fetch: its file, or the death of the result (announced on `changes`).
    fn settle(&self, id: u64, outcome: Result<Arc<Fetched>, SourceError>) {
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
    pub fn get(&self, id: u64, url: &str) -> Result<Arc<Fetched>, SourceError> {
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
    pub fn get_known(&self, id: u64) -> Result<Arc<Fetched>, SourceError> {
        let url = lock(&self.table)
            .slots
            .get(&id)
            .map(|slot| slot.url.clone())
            .ok_or(SourceError::Yandex(YandexError::Unexpected("no such result")))?;
        self.get(id, &url)
    }

    /// The results that died since the last call, each once.
    pub fn take_dead(&self) -> Vec<u64> {
        std::mem::take(&mut lock(&self.table).dead)
    }
}

/// The grid picture of `bytes`, an image this app decodes: a JPEG `height` px high (the panel's rows), made by the
/// gallery's thumbnail pipeline with one of the app's makers. The reason when the bytes are not such an image.
pub(crate) fn grid_picture(makers: &Makers, bytes: &[u8], height: usize) -> Result<Vec<u8>, &'static str> {
    if Format::sniff(bytes).is_none() {
        return Err("the file is not a picture");
    }
    makers
        .with_maker(|maker| Ok(maker.make(bytes, height, Turns::NONE).map_err(|_| "the picture does not decode")))
        .map_err(|_| "no maker")?
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
        fn fetch(&self, url: &str) -> Result<Download, SourceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.contains(&url) {
                return Err(YandexError::Status(404).into());
            }
            Ok(Download { bytes: b"picture".to_vec(), format: Some(Format::Jpeg) })
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
        assert_eq!(grid_picture(&Makers::new(), b"not a picture at all", 220), Err("the file is not a picture"));
    }

    fn images(fetch: Arc<Counting>, check: fn(&[u8]) -> Result<Vec<u8>, &'static str>) -> Arc<Images> {
        Images::new(fetch, Arc::new(check), Arc::new(Bus::new(16)))
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
        assert!(matches!(error, SourceError::Undecodable("no")), "{error:?}");
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
