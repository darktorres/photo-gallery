//! The pictures of a search's results. Each result's file is fetched once, from the server it is on, and kept for as long as
//! the search is shown: the grid shows it, and a save writes the same bytes. A file that can't be fetched, or that isn't
//! an image this app decodes, is dead: the result is taken off the grid (`Poll::dead`), and it is never saved. The
//! files are fetched as the results come in, a few at a time from each server (`pg_yandex::Gate`), so dead links are
//! known before they are shown. A result's size is its file's, as it decodes: one that is kept is reported with its size
//! once (`Poll::sized`), as a dead one is. Yandex's own figures for the sizes are not used (`pg_yandex::OtherSize` says
//! why).
//!
//! A scheduler thread starts the queued fetches as the servers have room for them (it is kicked whenever a slot frees
//! up, by any search), and a request for a result (`get`) joins the same queue at the front and waits for it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use pg_jobs::events::Bus;
use pg_model::format::Format;
use pg_model::orientation::Orientation;
use pg_model::size::RowHeight;
use pg_model::turns::Turns;
use pg_model::yandex::UnexpectedReason;
use pg_yandex::{Download, Fetcher, Gate, Kick, Permit, ResultUrl, Server, YandexError};

use super::source::SourceError;
use crate::makers::Makers;

/// Decodes at once: the bodies of the results being decoded are the ones in memory, as the old four fetchers allowed.
const DECODERS: usize = 4;

/// Bodies downloaded and not yet decoded, past which no new fetch starts: a bound on the memory of a search whose files
/// come faster than they decode.
const DECODE_BACKLOG: usize = 2 * DECODERS;

/// How long a request waits before it looks again for a free server or a scheduler that has failed.
const WAIT: Duration = Duration::from_millis(200);

/// How long the scheduler waits after a fetch thread could not be started, before it tries again.
const BACKOFF: Duration = Duration::from_millis(250);

/// Panics in a row that end the scheduler for good. A request still fetches its own result without it.
const MAX_PANICS: u32 = 3;

/// Fetches a result's file from the server it is on, holding `permit`, the slot of that server, until the body is read.
/// `cancel` is set when the images are stopped: a fetch that is under way is then cut short, and it ends as
/// [`SourceError::Closed`].
pub trait Fetch: Send + Sync {
    /// The gate whose slots the fetches take. The images subscribe to it, so a freed slot wakes their schedulers.
    fn gate(&self) -> &Arc<Gate>;

    /// Fetches the file at `url`, with the slot of its server.
    fn fetch(&self, url: &ResultUrl, permit: Permit, cancel: &AtomicBool) -> Result<Download, SourceError>;
}

impl Fetch for Fetcher {
    fn gate(&self) -> &Arc<Gate> {
        Fetcher::gate(self)
    }

    fn fetch(&self, url: &ResultUrl, permit: Permit, cancel: &AtomicBool) -> Result<Download, SourceError> {
        match self.download(url, permit, cancel) {
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

/// What [`Check`] makes of a file: its grid picture, the size of the image the file is, whether it is an animation, and
/// the EXIF orientation its tag holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The picture made for the grid.
    pub grid: Vec<u8>,
    /// The image's size, upright, as its bytes decode (width, height), in pixels.
    pub size: (u32, u32),
    /// Whether the file holds more than one frame (an animation), from its container.
    pub animated: bool,
    /// The EXIF orientation the file's tag holds (1 when it holds none).
    pub orientation: Orientation,
}

/// A result's file, kept: the file as Yandex's host gave it (what a save writes, and what the preview plays), its format
/// (a save's extension), its grid picture (a JPEG as high as the panel's rows, which the grid shows), its size, whether it
/// is an animation and its EXIF orientation.
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
    /// Whether the file is an animation.
    pub animated: bool,
    /// The EXIF orientation the file's tag holds.
    pub orientation: Orientation,
}

impl AsRef<[u8]> for Fetched {
    /// The file's bytes, as the host gave them: the animation player plays them without a copy.
    fn as_ref(&self) -> &[u8] {
        &self.file.bytes
    }
}

/// A result whose file came in: its id, its image's size, upright (width, height), in pixels, whether it is an
/// animation, and its EXIF orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measured {
    /// The result's id (`Hit::id`).
    pub id: u64,
    /// The size of the image its file is.
    pub size: (u32, u32),
    /// Whether its file is an animation.
    pub animated: bool,
    /// The EXIF orientation its file's tag holds.
    pub orientation: Orientation,
}

/// Where a result's file is.
enum State {
    /// Waiting for a server to have room for it.
    Queued,
    /// Being fetched now, by the scheduler or by a request that asked for it.
    Fetching,
    Ready(Arc<Fetched>),
    Dead(SourceError),
}

struct Slot {
    /// The address the file is at, when it is one the app fetches. A result without one is dead from the start.
    url: Option<ResultUrl>,
    state: State,
}

/// Whether the scheduler thread has been started, and whether it has failed for good.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scheduler {
    #[default]
    NotStarted,
    Running,
    /// It could not be started, or panicked too often: requests fetch their own results, and nothing else is started.
    Failed,
}

#[derive(Default)]
struct Table {
    slots: BTreeMap<u64, Slot>,
    /// The results waiting for their server to have room, in order.
    queue: VecDeque<u64>,
    /// The dead results, until `take_dead` reports them.
    dead: Vec<u64>,
    /// The results kept, with their sizes, until `take_sized` reports them.
    sized: Vec<Measured>,
    /// Set by [`Images::stop`]: nothing new is fetched, and a request for a file that is not kept gets `Closed`.
    stopped: bool,
    /// Bodies downloaded and not yet decoded.
    undecoded: usize,
    scheduler: Scheduler,
}

/// A fetch that has its slot: the result, its address and the permit of its server.
struct Job {
    id: u64,
    url: ResultUrl,
    permit: Permit,
}

/// What a pass of the scheduler did, for the scheduler (and for a request, which may need its own job back).
struct Pumped {
    /// A fetch thread could not be started: the scheduler pauses before it tries again.
    spawn_failed: bool,
    /// The job of the request that asked for the pass, when its fetch thread could not be started: the request fetches
    /// it itself, holding its slot.
    want_job: Option<Job>,
}

/// Which thread a spawn is for, so a test can make one kind fail and not the other.
#[derive(Clone, Copy, Debug)]
pub enum ThreadKind {
    /// The scheduler of an images set.
    Scheduler,
    /// A fetch of one result.
    Fetch,
}

/// How the images start their threads: [`thread::Builder`] in the app, a stand-in in the tests that make one fail.
type Spawner = Box<dyn Fn(ThreadKind, &str, Box<dyn FnOnce() + Send>) -> io::Result<()> + Send + Sync>;

fn thread_spawner() -> Spawner {
    Box::new(|_kind: ThreadKind, name: &str, work: Box<dyn FnOnce() + Send>| {
        thread::Builder::new().name(name.to_string()).spawn(work).map(drop)
    })
}

/// The files of one search's results, by result id.
pub struct Images {
    table: Mutex<Table>,
    /// Signalled when a file leaves `Fetching`, and when the scheduler fails.
    settled: Condvar,
    fetch: Arc<dyn Fetch>,
    check: Check,
    changes: Arc<Bus<()>>,
    /// Set by [`Images::stop`]: the fetches under way are cut short (`Fetch::fetch`), and the decodes waiting end.
    cancel: AtomicBool,
    /// Woken when a slot frees up, when a decode ends, and when a request queues a result.
    kick: Arc<Kick>,
    decoders: Decoders,
    spawn: Spawner,
    backoff: Duration,
}

/// The decodes running at once: a counting semaphore whose waiters can be cut short by the close.
#[derive(Default)]
struct Decoders {
    busy: Mutex<usize>,
    freed: Condvar,
}

/// A decode's place among the [`DECODERS`]: given back when it is dropped.
struct DecodeSlot<'a> {
    decoders: &'a Decoders,
}

fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock_busy(busy: &Mutex<usize>) -> MutexGuard<'_, usize> {
    busy.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Decoders {
    /// A place among the decodes, waited for while all are taken. Ends as `Closed` when `cancel` is set, which is checked
    /// under the lock before each wait; [`Decoders::wake`] is how a waiter already asleep learns of it.
    fn enter(&self, cancel: &AtomicBool) -> Result<DecodeSlot<'_>, SourceError> {
        let mut busy = lock_busy(&self.busy);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(SourceError::Closed);
            }
            if *busy < DECODERS {
                *busy += 1;
                return Ok(DecodeSlot { decoders: self });
            }
            busy = self.freed.wait(busy).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Wakes every waiter so that each checks the cancel flag. Takes the lock first, as [`pg_yandex::Gate::wake`] does.
    fn wake(&self) {
        let _busy = lock_busy(&self.busy);
        self.freed.notify_all();
    }
}

impl Drop for DecodeSlot<'_> {
    fn drop(&mut self) {
        {
            let mut busy = lock_busy(&self.decoders.busy);
            *busy = busy.saturating_sub(1);
        }
        self.decoders.freed.notify_one();
    }
}

/// A body downloaded and not yet decoded: counted while it is held, and the scheduler is kicked when it is decoded, since
/// the scheduler may have stopped fetching for the backlog.
struct Backlog<'a> {
    images: &'a Images,
}

impl<'a> Backlog<'a> {
    fn enter(images: &'a Images) -> Backlog<'a> {
        lock(&images.table).undecoded += 1;
        Backlog { images }
    }
}

impl Drop for Backlog<'_> {
    fn drop(&mut self) {
        {
            let mut t = lock(&self.images.table);
            t.undecoded = t.undecoded.saturating_sub(1);
        }
        self.images.kick.kick();
    }
}

/// A queued job whose fetch thread has not been started yet. If the pass that is starting it unwinds first, the job goes
/// back to the queue, so no slot is left `Fetching` with no thread.
struct Unstarted<'a> {
    images: &'a Images,
    cell: Arc<Mutex<Option<Job>>>,
    armed: bool,
}

impl Drop for Unstarted<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Some(job) = take_job(&self.cell)
        {
            self.images.requeue(job);
        }
    }
}

/// The job in a cell, taken out of it: the cell is shared with the thread that runs the job, and whichever takes it first
/// runs it.
fn take_job(cell: &Mutex<Option<Job>>) -> Option<Job> {
    cell.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// The error of an address the app does not fetch.
fn not_web_address() -> SourceError {
    SourceError::Yandex(YandexError::Unexpected(UnexpectedReason::NotWebAddress))
}

/// Marks a result dead, with its reason, and reports it as dead.
fn kill(t: &mut Table, id: u64, error: SourceError) {
    t.slots.insert(id, Slot { url: None, state: State::Dead(error) });
    t.dead.push(id);
}

/// Moves a queued result to the front of the queue: the next one to start, once its server has room.
fn promote(t: &mut Table, id: u64) {
    if let Some(pos) = t.queue.iter().position(|&queued| queued == id) {
        let _ = t.queue.remove(pos);
    }
    t.queue.push_front(id);
}

impl Images {
    /// The files of a search, fetched with `fetch` and checked with `check`. Each dead result, and each kept one (with
    /// its size), is announced on `changes`.
    pub fn new(fetch: Arc<dyn Fetch>, check: Check, changes: Arc<Bus<()>>) -> Arc<Images> {
        Self::with_spawner(fetch, check, changes, thread_spawner())
    }

    /// [`Images::new`], with the threads started by `spawn`.
    fn with_spawner(fetch: Arc<dyn Fetch>, check: Check, changes: Arc<Bus<()>>, spawn: Spawner) -> Arc<Images> {
        let kick = Kick::new();
        fetch.gate().subscribe(Arc::downgrade(&kick));
        Arc::new(Images {
            table: Mutex::default(),
            settled: Condvar::new(),
            fetch,
            check,
            changes,
            cancel: AtomicBool::new(false),
            kick,
            decoders: Decoders::default(),
            spawn,
            backoff: BACKOFF,
        })
    }

    /// Queues the results `(id, url)` for fetching, and starts the scheduler, which starts them as their servers have
    /// room. A result whose address is not one the app fetches is dead from the start. A result no fetch reaches is
    /// fetched when it is asked for ([`Images::get`]).
    pub fn prefetch(self: &Arc<Self>, results: Vec<(u64, String)>) {
        let mut died = false;
        {
            let mut t = lock(&self.table);
            for (id, text) in results {
                match ResultUrl::parse(&text) {
                    Some(url) => {
                        t.slots.insert(id, Slot { url: Some(url), state: State::Queued });
                        t.queue.push_back(id);
                    }
                    None => {
                        kill(&mut t, id, not_web_address());
                        died = true;
                    }
                }
            }
        }
        if died {
            self.changes.publish(());
        }
        self.ensure_scheduler();
        self.kick.kick();
    }

    /// Starts the scheduler thread, once: never after the images are stopped, and not again after it has failed.
    fn ensure_scheduler(self: &Arc<Self>) {
        {
            let mut t = lock(&self.table);
            if t.stopped || t.scheduler != Scheduler::NotStarted {
                return;
            }
            t.scheduler = Scheduler::Running;
        }
        let images = Arc::downgrade(self);
        let kick = Arc::clone(&self.kick);
        let started =
            (self.spawn)(ThreadKind::Scheduler, "yandex scheduler", Box::new(move || scheduler_loop(images, kick)));
        if started.is_err() {
            lock(&self.table).scheduler = Scheduler::Failed;
            self.settled.notify_all();
        }
    }

    /// Nothing new is fetched from now on: the scheduler ends, and a request for a file that is not kept gets
    /// [`SourceError::Closed`] at once. A download under way is cut short between its reads (`Fetch::fetch`). No thread
    /// is joined: a caller is not held up by a download.
    pub fn stop(&self) {
        lock(&self.table).stopped = true;
        self.cancel.store(true, Ordering::Relaxed);
        self.fetch.gate().wake();
        self.decoders.wake();
        self.kick.close();
        self.settled.notify_all();
    }

    /// How many results the search has (all of them, not only those still kept).
    pub fn count(&self) -> usize {
        lock(&self.table).slots.len()
    }

    /// The next result that can start: the first queued one whose server has a free slot, which takes that slot. A
    /// result whose server is full keeps its place, and so do the others of that server. `None` when nothing can start,
    /// the images are stopped, or the decodes are backed up. Lock order: the table, then the gate's.
    fn next_queued(&self) -> Option<Job> {
        let mut t = lock(&self.table);
        if t.stopped || t.undecoded >= DECODE_BACKLOG {
            return None;
        }
        let gate = self.fetch.gate();
        // A server refused in this scan is not asked again, so a scan costs one gate call per distinct server.
        let mut refused: BTreeSet<Server> = BTreeSet::new();
        let mut i = 0;
        loop {
            let &id = t.queue.get(i)?;
            let candidate = match t.slots.get(&id) {
                Some(Slot { url: Some(url), state: State::Queued }) => Some((url.server().clone(), url.clone())),
                _ => None,
            };
            // A result that is no longer queued (it was fetched, or died) leaves the queue.
            let Some((server, url)) = candidate else {
                let _ = t.queue.remove(i);
                continue;
            };
            if refused.contains(&server) {
                i += 1;
                continue;
            }
            match gate.try_acquire(&server) {
                Some(permit) => {
                    let _ = t.queue.remove(i);
                    if let Some(slot) = t.slots.get_mut(&id) {
                        slot.state = State::Fetching;
                    }
                    return Some(Job { id, url, permit });
                }
                None => {
                    refused.insert(server);
                    i += 1;
                }
            }
        }
    }

    /// A request's own result, taken out of the queue when its server has room, whatever its place in the queue. Used when
    /// the pass that a request runs could not start a fetch thread, so the request does not depend on the queue's order.
    fn claim(&self, id: u64) -> Option<Job> {
        let mut t = lock(&self.table);
        if t.stopped {
            return None;
        }
        let (server, url) = match t.slots.get(&id) {
            Some(Slot { url: Some(url), state: State::Queued }) => (url.server().clone(), url.clone()),
            _ => return None,
        };
        let permit = self.fetch.gate().try_acquire(&server)?;
        if let Some(pos) = t.queue.iter().position(|&queued| queued == id) {
            let _ = t.queue.remove(pos);
        }
        if let Some(slot) = t.slots.get_mut(&id) {
            slot.state = State::Fetching;
        }
        Some(Job { id, url, permit })
    }

    /// One pass of the scheduler, or of a request that waits for its own result: starts every job that can start. A
    /// fetch thread that cannot be started gives its job back, and the pass stops (`spawn_failed`); the job of `want`, the
    /// result the request waits for, goes back to the request instead, which fetches it on its own thread.
    fn pump(self: &Arc<Self>, want: Option<u64>) -> Pumped {
        let mut pumped = Pumped { spawn_failed: false, want_job: None };
        while let Some(job) = self.next_queued() {
            let id = job.id;
            let cell: Arc<Mutex<Option<Job>>> = Arc::new(Mutex::new(Some(job)));
            let mut unstarted = Unstarted { images: self, cell: Arc::clone(&cell), armed: true };
            let images = Arc::clone(self);
            let thread_cell = Arc::clone(&cell);
            let work: Box<dyn FnOnce() + Send> = Box::new(move || {
                if let Some(job) = take_job(&thread_cell) {
                    images.run(job);
                }
            });
            let started = (self.spawn)(ThreadKind::Fetch, &format!("yandex image {id}"), work);
            unstarted.armed = false;
            if started.is_err() {
                pumped.spawn_failed = true;
                if let Some(job) = take_job(&cell) {
                    if want == Some(job.id) {
                        pumped.want_job = Some(job);
                    } else {
                        self.requeue(job);
                    }
                }
                break;
            }
        }
        pumped
    }

    /// Gives a job that did not start back to the front of the queue, and its slot back to its server.
    fn requeue(&self, job: Job) {
        let Job { id, permit, .. } = job;
        {
            let mut t = lock(&self.table);
            if let Some(slot) = t.slots.get_mut(&id) {
                slot.state = State::Queued;
            }
            t.queue.push_front(id);
        }
        drop(permit);
    }

    /// Fetches the file of a job and keeps its outcome. Runs on a fetch thread, or on a request's own thread when the
    /// fetch thread could not start.
    fn run(&self, job: Job) {
        let Job { id, url, permit } = job;
        let outcome = self.fetch_caught(&url, permit);
        self.settle(id, outcome);
    }

    /// The file at `url`, fetched with `permit`, and kept only when the app decodes it: a format the model knows, which the
    /// check makes a grid picture of. A decode waits for one of the [`DECODERS`], and the body counts as backlog until it
    /// is decoded.
    fn fetch_checked(&self, url: &ResultUrl, permit: Permit) -> Result<Arc<Fetched>, SourceError> {
        let file = self.fetch.fetch(url, permit, &self.cancel)?;
        let _backlog = Backlog::enter(self);
        let _decode = self.decoders.enter(&self.cancel)?;
        let format = file.format.ok_or(SourceError::Undecodable("not a format this app knows"))?;
        let Picture { grid, size, animated, orientation } =
            (self.check)(&file.bytes).map_err(SourceError::Undecodable)?;
        Ok(Arc::new(Fetched { file, format, grid, size, animated, orientation }))
    }

    /// [`Images::fetch_checked`], where a panic (the decoder's, on a malformed file) ends the result as dead: a result
    /// left in `Fetching` would hold up every caller that waits for it.
    fn fetch_caught(&self, url: &ResultUrl, permit: Permit) -> Result<Arc<Fetched>, SourceError> {
        match pg_jobs::panics::catch(AssertUnwindSafe(|| self.fetch_checked(url, permit))) {
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
                    t.sized.push(Measured {
                        id,
                        size: fetched.size,
                        animated: fetched.animated,
                        orientation: fetched.orientation,
                    });
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

    /// The file of the result `id`, which is at `url`: kept when it is in, waited for when it is being fetched, and queued
    /// at the front otherwise, to be fetched as soon as its server has room. A dead result is an error. The caller waits
    /// only while the images are open: once they are stopped ([`Images::stop`]) it gets [`SourceError::Closed`] at once.
    pub fn get(self: &Arc<Self>, id: u64, url: &str) -> Result<Arc<Fetched>, SourceError> {
        self.request(id, ResultUrl::parse(url))
    }

    /// [`Images::get`], with the address already read (`None` when it is not one the app fetches).
    fn request(self: &Arc<Self>, id: u64, url: Option<ResultUrl>) -> Result<Arc<Fetched>, SourceError> {
        let mut died = false;
        {
            let mut t = lock(&self.table);
            let seen = match t.slots.get(&id) {
                Some(slot) => match &slot.state {
                    State::Ready(fetched) => Seen::Ready(fetched.clone()),
                    State::Dead(error) => Seen::Dead(error.clone()),
                    State::Queued => Seen::Queued,
                    State::Fetching => Seen::Fetching,
                },
                None => Seen::Unknown,
            };
            match seen {
                Seen::Ready(fetched) => return Ok(fetched),
                Seen::Dead(error) => return Err(error),
                _ if t.stopped => return Err(SourceError::Closed),
                Seen::Queued => promote(&mut t, id),
                Seen::Fetching => {}
                Seen::Unknown => match url {
                    Some(url) => {
                        t.slots.insert(id, Slot { url: Some(url), state: State::Queued });
                        t.queue.push_front(id);
                    }
                    None => {
                        kill(&mut t, id, not_web_address());
                        died = true;
                    }
                },
            }
        }
        if died {
            self.changes.publish(());
            return Err(not_web_address());
        }
        self.ensure_scheduler();
        self.kick.kick();
        self.await_result(id)
    }

    /// Waits for the result `id` to be kept or dead. While it is still queued, this thread starts what can start, itself
    /// (`pump`), so a request never depends on the scheduler: a failed scheduler, or fetch threads that cannot start, only
    /// slow it down. The result's own job, when its thread could not start, is run here, holding its slot.
    fn await_result(self: &Arc<Self>, id: u64) -> Result<Arc<Fetched>, SourceError> {
        loop {
            let queued = {
                let t = lock(&self.table);
                match t.slots.get(&id).map(|slot| &slot.state) {
                    Some(State::Ready(fetched)) => return Ok(fetched.clone()),
                    Some(State::Dead(error)) => return Err(error.clone()),
                    _ if t.stopped => return Err(SourceError::Closed),
                    Some(State::Queued) => true,
                    _ => false,
                }
            };
            if queued {
                let pumped = self.pump(Some(id));
                if let Some(job) = pumped.want_job {
                    self.run(job);
                } else if pumped.spawn_failed
                    && let Some(job) = self.claim(id)
                {
                    // Another result's fetch thread could not start, which stopped the pass before this one's turn.
                    self.run(job);
                }
            }
            let t = lock(&self.table);
            let _ = self.settled.wait_timeout(t, WAIT).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The file of the result `id` if it is kept, and `None` otherwise. Never waits: unlike [`Images::get`] it starts no
    /// fetch and waits for none, so a player's thread can take it without a download holding it up.
    pub fn kept(&self, id: u64) -> Option<Arc<Fetched>> {
        match lock(&self.table).slots.get(&id).map(|slot| &slot.state) {
            Some(State::Ready(fetched)) => Some(fetched.clone()),
            _ => None,
        }
    }

    /// The file of the result `id`, at the address it was queued with. An id that no search has is an error.
    pub fn get_known(self: &Arc<Self>, id: u64) -> Result<Arc<Fetched>, SourceError> {
        let known = lock(&self.table).slots.get(&id).map(|slot| slot.url.clone());
        match known {
            Some(url) => self.request(id, url),
            None => Err(SourceError::Yandex(YandexError::Unexpected(UnexpectedReason::NoResult))),
        }
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

impl Drop for Images {
    /// Ends the scheduler: it holds only a weak reference, so it would otherwise wait for a kick that never comes.
    fn drop(&mut self) {
        self.kick.close();
    }
}

/// What a request finds for its result, copied out of the table before it acts.
enum Seen {
    Ready(Arc<Fetched>),
    Dead(SourceError),
    Queued,
    Fetching,
    Unknown,
}

/// The scheduler's loop: waits to be kicked, then starts what can start. A pass that panics is logged by the panic hook
/// and the loop goes on; [`MAX_PANICS`] in a row end it, as `Failed`. It ends when the images are stopped or dropped.
fn scheduler_loop(images: Weak<Images>, kick: Arc<Kick>) {
    let mut panics: u32 = 0;
    loop {
        kick.wait();
        let Some(images) = images.upgrade() else { return };
        if lock(&images.table).stopped {
            return;
        }
        match pg_jobs::panics::catch(AssertUnwindSafe(|| images.pump(None))) {
            Ok(pumped) => {
                panics = 0;
                if pumped.spawn_failed {
                    kick.pause(images.backoff);
                }
            }
            Err(_) => {
                panics = panics.saturating_add(1);
                if panics >= MAX_PANICS {
                    lock(&images.table).scheduler = Scheduler::Failed;
                    images.settled.notify_all();
                    return;
                }
            }
        }
    }
}

/// The grid picture of `bytes`, an image this app decodes: a JPEG `height` px high (the panel's rows), made by the
/// gallery's thumbnail pipeline with one of the app's makers, and the image's upright size as the decode read it. The
/// reason when the bytes are not such an image.
pub(crate) fn grid_picture(makers: &Makers, bytes: &[u8], height: RowHeight) -> Result<Picture, &'static str> {
    let Some(format) = Format::sniff(bytes) else {
        return Err("the file is not a picture");
    };
    // The row's height in pixels, as the maker takes it.
    let px = pg_model::num::index(height.get());
    let made = makers
        .with_maker(|maker| maker.make_sized(bytes, px, Turns::NONE).map_err(|_| "the picture does not decode"))
        .map_err(|_| "no maker")?;
    made.map(|(grid, size)| Picture {
        grid,
        size,
        animated: pg_render::player::animated(bytes, format),
        orientation: pg_render::display::orientation_of(bytes),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use pg_model::format::Format;
    use pg_yandex::PER_SERVER;

    use super::*;

    /// A fetch that serves `body` for every URL except those in `fail`, and counts the fetches. Its gate is its own.
    struct Counting {
        calls: AtomicUsize,
        fail: Vec<&'static str>,
        gate: Arc<Gate>,
    }

    impl Fetch for Counting {
        fn gate(&self) -> &Arc<Gate> {
            &self.gate
        }

        fn fetch(&self, url: &ResultUrl, permit: Permit, _cancel: &AtomicBool) -> Result<Download, SourceError> {
            drop(permit);
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.contains(&url.original()) {
                return Err(YandexError::Status(404).into());
            }
            Ok(Download { bytes: b"picture".to_vec(), format: Some(Format::Jpeg) })
        }
    }

    fn counting(fail: Vec<&'static str>) -> Arc<Counting> {
        Arc::new(Counting { calls: AtomicUsize::new(0), fail, gate: Gate::new(PER_SERVER) })
    }

    fn accept(_: &[u8]) -> Result<Picture, &'static str> {
        Ok(Picture { grid: b"grid".to_vec(), size: (810, 1080), animated: false, orientation: Orientation::UPRIGHT })
    }

    fn reject(_: &[u8]) -> Result<Picture, &'static str> {
        Err("no")
    }

    fn changes() -> Arc<Bus<()>> {
        Arc::new(Bus::new(16))
    }

    fn images(fetch: Arc<dyn Fetch>, check: fn(&[u8]) -> Result<Picture, &'static str>) -> Arc<Images> {
        Images::new(fetch, Arc::new(check), changes())
    }

    /// Bytes that are no picture make no grid picture.
    #[test]
    fn a_file_that_is_no_picture_makes_no_grid_picture() {
        assert_eq!(
            grid_picture(&Makers::new(), b"not a picture at all", RowHeight::DEFAULT),
            Err("the file is not a picture")
        );
    }

    /// A result fetched in the background is kept: asking for it again fetches nothing.
    #[test]
    fn a_result_is_fetched_once_and_kept() {
        let fetch = counting(vec![]);
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
        let images = images(counting(vec!["https://gone.example/2.jpg"]), accept);
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().size, (810, 1080));
        assert!(images.get(2, "https://gone.example/2.jpg").is_err());
        assert_eq!(
            images.take_sized(),
            [Measured { id: 1, size: (810, 1080), animated: false, orientation: Orientation::UPRIGHT }]
        );
        assert!(images.take_sized().is_empty(), "reported once");
        images.get(1, "https://a.example/1.jpg").unwrap();
        assert!(images.take_sized().is_empty(), "a kept file asked for again is not reported again");
    }

    /// A file that can't be fetched is dead: asking for it fails, and the death is reported once.
    #[test]
    fn a_result_that_cannot_be_fetched_is_dead_and_reported_once() {
        let images = images(counting(vec!["https://gone.example/2.jpg"]), accept);
        images.prefetch(vec![(2, "https://gone.example/2.jpg".into())]);
        assert!(images.get(2, "https://gone.example/2.jpg").is_err());
        assert_eq!(images.take_dead(), vec![2]);
        assert!(images.take_dead().is_empty(), "reported once");
    }

    /// A file that is not an image this app decodes is dead, with the check's reason.
    #[test]
    fn a_file_that_does_not_decode_is_dead() {
        let images = images(counting(vec![]), reject);
        let error = images.get(3, "https://a.example/3.png").unwrap_err();
        assert!(matches!(error, SourceError::Undecodable("no")), "{error:?}");
        assert_eq!(images.take_dead(), vec![3]);
    }

    /// A result with no fetch yet is fetched when it is asked for, and by its address.
    #[test]
    fn a_result_asked_for_before_it_is_queued_is_fetched_by_its_url() {
        let fetch = counting(vec![]);
        let images = images(fetch.clone(), accept);
        assert_eq!(images.get_known(9).unwrap_err().to_string(), "unexpected Yandex response: NoResult");
        assert_eq!(images.get(9, "https://a.example/9.jpg").unwrap().file.bytes, b"picture");
        assert_eq!(images.get_known(9).unwrap().file.bytes, b"picture");
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 1);
    }

    /// An address that is not a web address is dead from the start, once, for `prefetch` and for `get` alike.
    #[test]
    fn a_result_with_no_web_address_is_dead_at_once() {
        let images = images(counting(vec![]), accept);
        images.prefetch(vec![(5, "ftp://a.example/x.jpg".into())]);
        assert_eq!(images.take_dead(), vec![5]);
        let error = images.get(6, "https://user@a.example/x.jpg").unwrap_err();
        assert!(matches!(error, SourceError::Yandex(YandexError::Unexpected(UnexpectedReason::NotWebAddress))));
        assert_eq!(images.take_dead(), vec![6]);
    }

    /// A fetch that holds its slot until `hold` is cleared, and records the most fetches in flight at once.
    struct Holding {
        gate: Arc<Gate>,
        hold: Arc<AtomicBool>,
        in_flight: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Fetch for Holding {
        fn gate(&self) -> &Arc<Gate> {
            &self.gate
        }

        fn fetch(&self, _url: &ResultUrl, permit: Permit, _cancel: &AtomicBool) -> Result<Download, SourceError> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            while self.hold.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            drop(permit);
            Ok(Download { bytes: b"picture".to_vec(), format: Some(Format::Jpeg) })
        }
    }

    fn holding(gate: Arc<Gate>, hold: Arc<AtomicBool>) -> Arc<Holding> {
        Arc::new(Holding { gate, hold, in_flight: AtomicUsize::new(0), peak: AtomicUsize::new(0) })
    }

    /// Ten results of one server are fetched at most [`PER_SERVER`] at a time, and that many at once.
    #[test]
    fn one_server_is_fetched_from_at_most_its_limit_at_once() {
        let hold = Arc::new(AtomicBool::new(true));
        let fetch = holding(Gate::new(PER_SERVER), hold.clone());
        let images = images(fetch.clone(), accept);
        let results: Vec<(u64, String)> = (1..=10).map(|id| (id, format!("https://a.example/{id}.jpg"))).collect();
        images.prefetch(results);
        let started = Instant::now();
        while fetch.peak.load(Ordering::SeqCst) < PER_SERVER && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fetch.peak.load(Ordering::SeqCst), PER_SERVER, "the limit is reached, and not passed");
        hold.store(false, Ordering::SeqCst);
        for id in 1..=10 {
            assert!(images.get(id, &format!("https://a.example/{id}.jpg")).is_ok());
        }
        assert_eq!(fetch.peak.load(Ordering::SeqCst), PER_SERVER);
    }

    /// Images that share a gate: a result queued behind the other images' fetch starts once that fetch ends, without
    /// anything asking for it.
    #[test]
    fn a_slot_freed_by_another_search_starts_a_queued_result() {
        let gate = Gate::new(1);
        let hold = Arc::new(AtomicBool::new(true));
        let first = images(holding(gate.clone(), hold.clone()), accept);
        let second_hold = Arc::new(AtomicBool::new(false));
        let second = images(holding(gate.clone(), second_hold), accept);
        first.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        std::thread::sleep(Duration::from_millis(50));
        second.prefetch(vec![(2, "https://a.example/2.jpg".into())]);
        let (tx, rx) = mpsc::channel();
        let waiting = second.clone();
        std::thread::spawn(move || {
            let _ = tx.send(waiting.get(2, "https://a.example/2.jpg").is_ok());
        });
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "the other search holds the slot");
        hold.store(false, Ordering::SeqCst);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
    }

    /// The scheduler cannot start: a request starts its own result all the same.
    #[test]
    fn a_failed_scheduler_does_not_stop_a_request() {
        let spawn: Spawner = Box::new(|kind: ThreadKind, name: &str, work: Box<dyn FnOnce() + Send>| match kind {
            ThreadKind::Scheduler => Err(io::Error::other("no thread for the scheduler")),
            ThreadKind::Fetch => thread::Builder::new().name(name.to_string()).spawn(work).map(drop),
        });
        let images = Images::with_spawner(counting(vec![]), Arc::new(accept), changes(), spawn);
        images.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().file.bytes, b"picture");
    }

    /// No thread can start at all: a request still runs its own fetch, inline.
    #[test]
    fn a_request_fetches_inline_when_no_thread_can_start() {
        let spawn: Spawner =
            Box::new(|_: ThreadKind, _: &str, _: Box<dyn FnOnce() + Send>| Err(io::Error::other("no threads left")));
        let images = Images::with_spawner(counting(vec![]), Arc::new(accept), changes(), spawn);
        images.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().file.bytes, b"picture");
        assert_eq!(images.take_dead(), Vec::<u64>::new());
    }

    /// A request that is waiting when the images close ends with `Closed`, and nothing is fetched after the close.
    #[test]
    fn a_stopped_image_set_gives_closed_to_a_waiting_request() {
        let gate = Gate::new(1);
        let hold = Arc::new(AtomicBool::new(true));
        let images = images(holding(gate.clone(), hold.clone()), accept);
        images.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        let (tx, rx) = mpsc::channel();
        let waiting = images.clone();
        std::thread::spawn(move || {
            let _ = tx.send(waiting.get(2, "https://a.example/2.jpg").err());
        });
        std::thread::sleep(Duration::from_millis(50));
        images.stop();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(10)), Ok(Some(SourceError::Closed))));
        hold.store(false, Ordering::SeqCst);
    }

    /// A check that holds every decode until `hold` is cleared.
    fn holding_check(hold: Arc<AtomicBool>) -> Check {
        Arc::new(move |_: &[u8]| {
            while hold.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(Picture { grid: b"grid".to_vec(), size: (1, 1), animated: false, orientation: Orientation::UPRIGHT })
        })
    }

    /// While the decodes are held, the scheduler stops starting fetches once the backlog is full: the fetches started are
    /// the backlog, plus at most the fetches that were already in flight.
    #[test]
    fn the_decode_backlog_bounds_the_fetches_started() {
        let hold = Arc::new(AtomicBool::new(true));
        let fetch = counting(vec![]);
        let images = Images::with_spawner(fetch.clone(), holding_check(hold.clone()), changes(), thread_spawner());
        let results: Vec<(u64, String)> = (1..=20).map(|id| (id, format!("https://a.example/{id}.jpg"))).collect();
        images.prefetch(results);
        std::thread::sleep(Duration::from_millis(300));
        let started = fetch.calls.load(Ordering::SeqCst);
        assert!(
            (DECODE_BACKLOG..=DECODE_BACKLOG + PER_SERVER).contains(&started),
            "{started} fetches started while the decodes are held"
        );
        hold.store(false, Ordering::SeqCst);
        for id in 1..=20 {
            assert!(images.get(id, &format!("https://a.example/{id}.jpg")).is_ok());
        }
    }

    /// A fetch that panics while `explode` is set, from the gate the scheduler asks for on each pass.
    struct Exploding {
        gate: Arc<Gate>,
        explode: Arc<AtomicBool>,
    }

    impl Fetch for Exploding {
        fn gate(&self) -> &Arc<Gate> {
            assert!(!self.explode.load(Ordering::SeqCst), "the gate broke");
            &self.gate
        }

        fn fetch(&self, _url: &ResultUrl, permit: Permit, _cancel: &AtomicBool) -> Result<Download, SourceError> {
            drop(permit);
            Ok(Download { bytes: b"picture".to_vec(), format: Some(Format::Jpeg) })
        }
    }

    /// Three scheduler passes in a row that panic end the scheduler as failed; a request still fetches its own result.
    #[test]
    fn three_panicking_passes_fail_the_scheduler_and_a_request_still_fetches() {
        let explode = Arc::new(AtomicBool::new(false));
        let fetch = Arc::new(Exploding { gate: Gate::new(PER_SERVER), explode: explode.clone() });
        let images = images(fetch, accept);
        // Armed only now: the images ask the gate once when they are made, and that must not panic.
        explode.store(true, Ordering::SeqCst);
        images.prefetch(vec![(1, "https://a.example/1.jpg".into())]);
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(50));
            images.kick.kick();
        }
        let started = Instant::now();
        while lock(&images.table).scheduler != Scheduler::Failed && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(lock(&images.table).scheduler, Scheduler::Failed);
        explode.store(false, Ordering::SeqCst);
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().file.bytes, b"picture");
    }

    /// A fetch thread that cannot start leaves its result queued, not dead: nothing runs it until a request does, and a
    /// request runs its own result even when it is not first in the queue.
    #[test]
    fn a_fetch_thread_that_cannot_start_leaves_its_result_queued_for_a_request() {
        let spawn: Spawner = Box::new(|kind: ThreadKind, name: &str, work: Box<dyn FnOnce() + Send>| match kind {
            ThreadKind::Scheduler => thread::Builder::new().name(name.to_string()).spawn(work).map(drop),
            ThreadKind::Fetch => Err(io::Error::other("no thread for a fetch")),
        });
        let fetch = counting(vec![]);
        let images = Images::with_spawner(fetch.clone(), Arc::new(accept), changes(), spawn);
        images.prefetch(vec![
            (1, "https://a.example/1.jpg".into()),
            (2, "https://a.example/2.jpg".into()),
            (3, "https://a.example/3.jpg".into()),
        ]);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 0, "nothing was run: no thread would run it");
        assert!(images.take_dead().is_empty(), "a result that cannot start is not dead");
        assert_eq!(images.get(2, "https://a.example/2.jpg").unwrap().file.bytes, b"picture");
        assert_eq!(images.get(1, "https://a.example/1.jpg").unwrap().file.bytes, b"picture");
    }

    /// Dropping the images ends their scheduler: it waits on the kick, which is closed then.
    #[test]
    fn dropping_the_images_closes_their_kick() {
        let images = images(counting(vec![]), accept);
        let kick = Arc::clone(&images.kick);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            kick.wait();
            let _ = tx.send(());
        });
        drop(images);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(()));
    }
}
