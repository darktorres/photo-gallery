//! The pictures of a search's results. Each result's file is fetched once, from the host it is on, and kept for as long as
//! the search is shown: the grid shows it, and a save writes the same bytes. A file that can't be fetched, or that isn't
//! an image this app decodes, is dead: the result is taken off the grid (`Poll::dead`), and it is never saved. The
//! files are fetched as the results come in, a few at a time, so dead links are known before they are shown. A result's
//! size is its file's, as it decodes: one that is kept is reported with its size once (`Poll::sized`), as a dead one is.
//! Yandex's own figures for the sizes are not used (`pg_yandex::OtherSize` says why).

use std::collections::{BTreeMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use pg_jobs::events::Bus;
use pg_model::format::Format;
use pg_model::size::RowHeight;
use pg_model::turns::Turns;
use pg_yandex::{Download, Fetcher, YandexError};

use super::source::SourceError;
use crate::makers::Makers;

/// How many files are fetched at once.
const FETCHERS: usize = 4;

/// Fetches a result's file from the host it is on. `cancel` is set when the images are stopped: a fetch that is under way
/// is then cut short, and it ends as [`SourceError::Closed`].
pub trait Fetch: Send + Sync {
    /// Fetches the file at `url`.
    fn fetch(&self, url: &str, cancel: &AtomicBool) -> Result<Download, SourceError>;
}

impl Fetch for Fetcher {
    fn fetch(&self, url: &str, cancel: &AtomicBool) -> Result<Download, SourceError> {
        match self.download(url, cancel) {
            Ok(download) => Ok(download),
            // A download the close cut short is not a file that failed: the caller is told the images are closed.
            Err(_) if cancel.load(Ordering::Relaxed) => Err(SourceError::Closed),
            Err(e) => Err(e.into()),
        }
    }
}

/// Makes the grid picture of bytes that are an image this app decodes, with the image's size; the reason when they are
/// not. The app's is `grid_picture`, with its makers and the panel's row height.
pub type Check = Arc<dyn Fn(&[u8]) -> Result<Picture, &'static str> + Send + Sync>;

/// What [`Check`] makes of a file: its grid picture, and the size of the image the file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The picture made for the grid.
    pub grid: Vec<u8>,
    /// The image's size, upright, as its bytes decode (width, height), in pixels.
    pub size: (u32, u32),
}

/// A result's file, kept: the file as Yandex's host gave it (what a save writes), its format (a save's extension), its
/// grid picture (a JPEG as high as the panel's rows, which the grid shows) and its size.
#[derive(Debug)]
pub struct Fetched {
    /// The downloaded file.
    pub file: Download,
    /// Its format.
    pub format: Format,
    /// The picture made for the grid.
    pub grid: Vec<u8>,
    /// The image's size, upright, as the file decodes (width, height), in pixels.
    pub size: (u32, u32),
}

/// A result whose file came in: its id, and its image's size, upright (width, height), in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The result's id (`Hit::id`).
    pub id: u64,
    /// The size of the image its file is.
    pub size: (u32, u32),
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
    /// The results kept, with their sizes, until `take_sized` reports them.
    sized: Vec<Measured>,
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
    /// Set by [`Images::stop`]: the fetches under way are cut short (`Fetch::fetch`).
    cancel: AtomicBool,
}

fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Images {
    /// The files of a search, fetched with `fetch` and checked with `check`. Each dead result, and each kept one (with
    /// its size), is announced on `changes`.
    pub fn new(fetch: Arc<dyn Fetch>, check: Check, changes: Arc<Bus<()>>) -> Arc<Images> {
        Arc::new(Images {
            table: Mutex::default(),
            settled: Condvar::new(),
            fetch,
            check,
            changes,
            cancel: AtomicBool::new(false),
        })
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
            // A fetcher is optional parallelism: a thread that can't start leaves its results queued, and `get` fetches
            // them on demand. So the failure is not reported (unlike a search or a save, see `Search::spawn_worker`).
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

    /// The fetchers take no more results, and nothing is fetched on demand any more. A caller waiting for a file gets
    /// [`SourceError::Closed`] at once, and a download under way is cut short between its reads (`Fetch::fetch`).
    pub fn stop(&self) {
        lock(&self.table).stopped = true;
        self.cancel.store(true, Ordering::Relaxed);
        self.settled.notify_all();
    }

    /// How many results the search has (all of them, not only those still kept).
    pub fn count(&self) -> usize {
        lock(&self.table).slots.len()
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
        let file = self.fetch.fetch(url, &self.cancel)?;
        let format = file.format.ok_or(SourceError::Undecodable("not a format this app knows"))?;
        let Picture { grid, size } = (self.check)(&file.bytes).map_err(SourceError::Undecodable)?;
        Ok(Arc::new(Fetched { file, format, grid, size }))
    }

    /// [`Images::fetch_checked`], where a panic (the decoder's, on a malformed file) ends the result as dead: a result
    /// left in `Fetching` would hold up every caller that waits for it.
    fn fetch_caught(&self, url: &str) -> Result<Arc<Fetched>, SourceError> {
        match pg_jobs::panics::catch(AssertUnwindSafe(|| self.fetch_checked(url))) {
            Ok(outcome) => outcome,
            Err(_) => Err(SourceError::Undecodable("the decoder stopped on this file")),
        }
    }

    /// Keeps the outcome of a fetch: its file and its size, or the death of the result. Either is announced on
    /// `changes`.
    fn settle(&self, id: u64, outcome: Result<Arc<Fetched>, SourceError>) {
        {
            let mut t = lock(&self.table);
            let state = match outcome {
                Ok(fetched) => {
                    t.sized.push(Measured { id, size: fetched.size });
                    State::Ready(fetched)
                }
                // Cut short by the close: the file is neither kept nor dead, and nothing is fetched for it any more.
                Err(SourceError::Closed) => State::Queued,
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
        self.changes.publish(());
    }

    /// The file of the result `id`, which is at `url`: kept when it is in, waited for when a fetcher is on it, and
    /// fetched now otherwise. A dead result is an error. The fetch runs on a thread of its own, and the caller waits
    /// for it only while the images are open: once they are stopped ([`Images::stop`]) the caller gets
    /// [`SourceError::Closed`] at once, so a closing app is not held up by a download.
    pub fn get(self: &Arc<Self>, id: u64, url: &str) -> Result<Arc<Fetched>, SourceError> {
        let mut t = lock(&self.table);
        loop {
            match t.slots.get(&id).map(|slot| &slot.state) {
                Some(State::Ready(fetched)) => return Ok(fetched.clone()),
                Some(State::Dead(error)) => return Err(error.clone()),
                // Closed: nothing new is fetched, and a file in flight is not waited for.
                _ if t.stopped => return Err(SourceError::Closed),
                // A fetcher or another caller has it: wait for it to settle.
                Some(State::Fetching) => {}
                Some(State::Queued) | None => break,
            }
            t = self.settled.wait(t).unwrap_or_else(PoisonError::into_inner);
        }
        t.slots.insert(id, Slot { url: url.to_string(), state: State::Fetching });
        drop(t);
        let images = Arc::clone(self);
        let fetching_url = url.to_string();
        let spawned = thread::Builder::new().name(format!("yandex fetch {id}")).spawn(move || {
            let outcome = images.fetch_caught(&fetching_url);
            images.settle(id, outcome);
        });
        if spawned.is_err() {
            // No thread for it: the file is fetched here, as it was before (the wait below then finds it settled).
            let outcome = self.fetch_caught(url);
            self.settle(id, outcome);
        }
        let mut t = lock(&self.table);
        loop {
            match t.slots.get(&id).map(|slot| &slot.state) {
                Some(State::Ready(fetched)) => return Ok(fetched.clone()),
                Some(State::Dead(error)) => return Err(error.clone()),
                _ if t.stopped => return Err(SourceError::Closed),
                _ => {}
            }
            t = self.settled.wait(t).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The file of the result `id`, at the URL it was queued with. An id that no search has is an error.
    pub fn get_known(self: &Arc<Self>, id: u64) -> Result<Arc<Fetched>, SourceError> {
        let url = lock(&self.table)
            .slots
            .get(&id)
            .map(|slot| slot.url.clone())
            .ok_or(SourceError::Yandex(YandexError::Unexpected(pg_model::yandex::UnexpectedReason::NoResult)))?;
        self.get(id, &url)
    }

    /// The results that died since the last call, each once.
    pub fn take_dead(&self) -> Vec<u64> {
        std::mem::take(&mut lock(&self.table).dead)
    }

    /// The results whose files came in since the last call, with their sizes, each once.
    pub fn take_sized(&self) -> Vec<Measured> {
        std::mem::take(&mut lock(&self.table).sized)
    }
}

/// The grid picture of `bytes`, an image this app decodes: a JPEG `height` px high (the panel's rows), made by the
/// gallery's thumbnail pipeline with one of the app's makers, and the image's upright size as the decode read it. The
/// reason when the bytes are not such an image.
pub(crate) fn grid_picture(makers: &Makers, bytes: &[u8], height: RowHeight) -> Result<Picture, &'static str> {
    if Format::sniff(bytes).is_none() {
        return Err("the file is not a picture");
    }
    // The row's height in pixels, as the maker takes it.
    let px = pg_model::num::index(height.get());
    let made = makers
        .with_maker(|maker| maker.make_sized(bytes, px, Turns::NONE).map_err(|_| "the picture does not decode"))
        .map_err(|_| "no maker")?;
    made.map(|(grid, size)| Picture { grid, size })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use pg_model::format::Format;

    use super::*;

    /// Serves `body` for every URL except those in `fail`, and counts the fetches.
    struct Counting {
        calls: AtomicUsize,
        fail: Vec<&'static str>,
    }

    impl Fetch for Counting {
        fn fetch(&self, url: &str, _cancel: &AtomicBool) -> Result<Download, SourceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.contains(&url) {
                return Err(YandexError::Status(404).into());
            }
            Ok(Download { bytes: b"picture".to_vec(), format: Some(Format::Jpeg) })
        }
    }

    fn accept(_: &[u8]) -> Result<Picture, &'static str> {
        Ok(Picture { grid: b"grid".to_vec(), size: (810, 1080) })
    }

    fn reject(_: &[u8]) -> Result<Picture, &'static str> {
        Err("no")
    }

    /// Bytes that are no picture make no grid picture.
    #[test]
    fn a_file_that_is_no_picture_makes_no_grid_picture() {
        assert_eq!(
            grid_picture(&Makers::new(), b"not a picture at all", RowHeight::DEFAULT),
            Err("the file is not a picture")
        );
    }

    fn images(fetch: Arc<Counting>, check: fn(&[u8]) -> Result<Picture, &'static str>) -> Arc<Images> {
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

    /// A kept file is reported once with the size its bytes decode to (the check's), and a dead one is not.
    #[test]
    fn a_kept_result_is_reported_once_with_its_files_size() {
        let fetch = Arc::new(Counting { calls: AtomicUsize::new(0), fail: vec!["https://gone.example/2.jpg"] });
        let images = images(fetch, accept);
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().size, (810, 1080));
        assert!(images.get(2, "https://gone.example/2.jpg").is_err());
        assert_eq!(images.take_sized(), [Measured { id: 1, size: (810, 1080) }]);
        assert!(images.take_sized().is_empty(), "reported once");
        images.get(1, "https://a.example/1.jpg").unwrap();
        assert!(images.take_sized().is_empty(), "a kept file asked for again is not reported again");
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
        assert_eq!(images.get_known(9).unwrap_err().to_string(), "unexpected Yandex response: NoResult");
        assert_eq!(images.get(9, "https://a.example/9.jpg").unwrap().file.bytes, b"picture");
        assert_eq!(images.get_known(9).unwrap().file.bytes, b"picture");
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 1);
    }
}
