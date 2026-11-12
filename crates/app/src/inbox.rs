//! The inbox (docs/NOTIFICATIONS.md): what the app tells the user, kept in the catalog until they dismiss it. A service:
//! it holds the catalog's path, and its own connection on its own thread; it never reaches the `App`.
//!
//! Every producer posts a [`Notice`] ([`Inbox::post`]), from any thread: the notice is logged there and then (so a
//! notice the inbox can't keep is still in the log), and queued for the writer. The writer keeps it as its lifecycle says
//! (`pg_catalog::inbox`), and after every change it reads the inbox and publishes what it lists ([`InboxEvent::Changed`]),
//! with a [`InboxEvent::Toast`] for each entry that is news (the toast rule, below). What another process changes (the
//! command line, the job's ledger writes) is found by the catalog's `data_version`, looked at every [`POLL`] and when
//! poked ([`Inbox::poke`]). The window's marks and dismissals are queued the same way; it never reads the catalog for
//! the inbox.
//!
//! **The work log.** What is queued is one `Work`, under one lock with a condvar, shared by the handles and the writer:
//! a wake is never lost and a close never blocks. Every op takes a sequence number under that lock, and the writer
//! applies what it takes in that order, across every kind of op (a thumbnail's failure reported before the user's Retry
//! is written before it, so the Retry relists it). What `Work` holds:
//! - Events: a bounded queue ([`EVENTS`]); one it has no room for is counted, and a [`Notice::NoticesLost`] says how
//!   many after the next write that goes through.
//! - States' posts, replaces and clears, which supersede the waiting ones they cover: a post replaces the waiting post
//!   of its key and carries its repeats forward (so the count never under-counts); a replace or a clear removes every
//!   waiting op inside its scope. Bounded by the keys and scopes waiting.
//! - Marks, dismissals and the Retry: at human rate, never dropped.
//! - The window's thumbnail reports, the latest per file.
//! - The flags: whether the inbox is viewed (the latest wins: the ledgers' counts are read only then), a poke, the stop.
//!
//! **Failures.** The writer's connection waits [`BUSY_TIMEOUT`] for a lock another connection holds; a write that is
//! still busy is tried again, in order, until it goes through, so a scan's batch or an import's flush only delays it.
//! Any other failure (`SQLITE_LOCKED` included: a conflict inside the connection, which trying again could loop on) is
//! not tried again: a lost notice is counted, and a mark or a dismissal that fails is logged and the inbox is read again,
//! which undoes the window's optimistic change. At `Inbox::close` the writer writes what is queued, trying a busy
//! catalog for at most [`SHUTDOWN_DRAIN`], and logs what is left.
//!
//! **The toast rule.** An Event toasts when it is new. A state toasts when it becomes unread (new, posted again with new
//! data after it was read or dismissed), or when its revision rises while it is unread, at most once per entry per
//! [`LOG_AGAIN`] (a run of failing files toasts once, and the entry shows the latest). A ledger toasts when it turns
//! unread. A transient notice (feedback on what the user just did) is only logged and toasted.
//!
//! A pass's failure ledger (`pg_catalog::ledger`) is an entry of its own, listed with the rows. The writer also keeps
//! the thumbnails' ledger for the thumbnails the window makes ([`Inbox::thumb_failed`], [`Inbox::thumb_made`]), and
//! carries out the user's Retry ([`Inbox::retry`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pg_catalog::Catalog;
use pg_catalog::inbox::{Entry, EntryId, Mark};
use pg_catalog::ledger::Failure;
use pg_jobs::NoticeSink;
use pg_jobs::events::{Bus, Subscription};
use pg_model::ids::FileId;
use pg_model::notice::{FilePass, Key, KeyBuf, Level, Lifecycle, Notice};

use crate::error::{Result, failed};

/// How many Events wait for the writer before one is counted as lost (a catalog locked for long by another process).
pub const EVENTS: usize = 1024;

/// How often the writer looks for what another process changed.
pub const POLL: Duration = Duration::from_secs(30);

/// How many entries the inbox lists, newest first.
pub const LISTED: usize = 200;

/// A kept notice posted again within this long is not logged again (an edit failing at every key); a state's new
/// revision is not toasted again within it either.
pub const LOG_AGAIN: Duration = Duration::from_secs(60);

/// How long a write of the inbox's waits for a lock another connection holds before it is tried again: the writer
/// looks for a stop between tries, so a close is seen within about this long.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(100);

/// The pause between two tries of a busy write (a close or new work ends it early).
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// How long the writer goes on writing what is queued after `Inbox::close`, while the catalog stays busy.
pub const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);

/// After this many busy tries in a row of one write (about 7 s) the writer logs that it is waiting.
const BUSY_WARN: u32 = 50;

/// What the inbox lists now: its entries, newest first, and how many are unread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub unread: u64,
}

/// What the inbox tells its subscribers.
#[derive(Clone, Debug)]
pub enum InboxEvent {
    /// The inbox changed: what it lists now.
    Changed(Arc<Listing>),
    /// A notice to show now: a new entry, one posted again that says something new, or a transient notice.
    Toast(Notice),
}

/// A kept state's change, waiting for the writer. Each supersedes the waiting ones it covers (`Work::state`).
enum StateOp {
    /// Post `notice` (kept under `key`), `repeats` times: the waiting posts of its key are folded in.
    Post { key: KeyBuf, notice: Notice, repeats: u64, at: i64 },
    /// The kept notices of `scope` are now `notices` (never empty: that is a `Clear`).
    Replace { scope: KeyBuf, notices: Vec<Notice>, at: i64 },
    /// The kept notices of `scope` no longer hold.
    Clear { scope: KeyBuf },
}

impl StateOp {
    /// Whether everything this op touches is inside `scope`, so a later replace or clear of `scope` makes it moot.
    fn within(&self, scope: &KeyBuf) -> bool {
        match self {
            StateOp::Post { key, .. } => covers(scope, key),
            StateOp::Replace { scope: inner, .. } | StateOp::Clear { scope: inner } => covers(scope, inner),
        }
    }
}

/// Whether `scope` covers `key` (a key, or a narrower scope): the same kind and folder, and the same part unless the
/// scope names every part. The folders are compared as given, so two paths in one library folder are not found to
/// cover each other: such ops are then both applied, in order, which is what they would be anyway.
fn covers(scope: &KeyBuf, key: &KeyBuf) -> bool {
    scope.code == key.code && scope.root == key.root && (scope.part.is_none() || scope.part == key.part)
}

/// What the user does to the inbox: never dropped.
enum Mutation {
    /// Mark these entries read, as they were displayed (one mark, or "Mark all read").
    MarkRead(Vec<EntryId>),
    /// Dismiss these entries, as they were displayed (one, or "Clear all").
    Dismiss(Vec<EntryId>),
    /// The user's Retry of a pass's failed files, then what follows it once written (the job woken).
    Retry(FilePass, Box<dyn FnOnce() + Send>),
}

/// A thumbnail the window asked for: it couldn't be made, or it was.
enum Thumb {
    Failed(Failure),
    Made,
}

/// An op the writer took, in sequence order.
enum Op {
    Event { notice: Notice, at: i64 },
    State(StateOp),
    Mutation(Mutation),
    Thumb(FileId, Thumb),
}

/// What is queued for the writer, under one lock (see the module's header).
#[derive(Default)]
struct Work {
    /// The last sequence number given.
    next: u64,
    events: VecDeque<(u64, Notice, i64)>,
    /// Events the queue had no room for, since the writer last took it.
    overflow: u64,
    states: Vec<(u64, StateOp)>,
    mutations: Vec<(u64, Mutation)>,
    /// The latest report per file.
    thumbs: BTreeMap<FileId, (u64, Thumb)>,
    /// Whether the inbox is viewed, when it was set since the writer last took it (the latest wins).
    viewing: Option<bool>,
    poke: bool,
    /// The inbox is closed: nothing more is queued, and the writer ends once it has written what is.
    stop: bool,
    /// The last sequence number the writer has applied, and read the inbox after: what `Inbox::idle` and a test
    /// wait on.
    done: u64,
}

/// What the writer takes from `Work` at once.
struct Batch {
    ops: Vec<Op>,
    overflow: u64,
    viewing: Option<bool>,
    stop: bool,
    /// The last sequence number given when it was taken.
    last: u64,
}

impl Work {
    fn seq(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Queue an Event. 0 when it is queued; else the queue is full, and how many it had no room for since the writer
    /// last took it, this one included.
    fn event(&mut self, notice: Notice, at: i64) -> u64 {
        if self.events.len() >= EVENTS {
            self.overflow += 1;
            return self.overflow;
        }
        let seq = self.seq();
        self.events.push_back((seq, notice, at));
        0
    }

    /// Queue a state's op, superseding the waiting ones it covers.
    fn state(&mut self, mut op: StateOp) {
        match &mut op {
            StateOp::Post { key, repeats, .. } => {
                let same =
                    self.states.iter().position(|(_, s)| matches!(s, StateOp::Post { key: k, .. } if *k == *key));
                if let Some(at) = same
                    && let (_, StateOp::Post { repeats: before, .. }) = self.states.remove(at)
                {
                    *repeats = repeats.saturating_add(before);
                }
            }
            StateOp::Replace { scope, .. } | StateOp::Clear { scope } => {
                let scope = &*scope;
                self.states.retain(|(_, s)| !s.within(scope));
            }
        }
        let seq = self.seq();
        self.states.push((seq, op));
    }

    fn mutation(&mut self, mutation: Mutation) {
        let seq = self.seq();
        self.mutations.push((seq, mutation));
    }

    fn thumb(&mut self, file: FileId, thumb: Thumb) {
        let seq = self.seq();
        self.thumbs.insert(file, (seq, thumb));
    }

    fn has_work(&self) -> bool {
        !self.events.is_empty()
            || !self.states.is_empty()
            || !self.mutations.is_empty()
            || !self.thumbs.is_empty()
            || self.overflow > 0
            || self.viewing.is_some()
            || self.poke
    }

    /// Everything queued, in sequence order, and the flags.
    fn take(&mut self) -> Batch {
        let mut ops: Vec<(u64, Op)> = Vec::new();
        ops.extend(self.events.drain(..).map(|(seq, notice, at)| (seq, Op::Event { notice, at })));
        ops.extend(self.states.drain(..).map(|(seq, op)| (seq, Op::State(op))));
        ops.extend(self.mutations.drain(..).map(|(seq, m)| (seq, Op::Mutation(m))));
        ops.extend(std::mem::take(&mut self.thumbs).into_iter().map(|(file, (seq, t))| (seq, Op::Thumb(file, t))));
        ops.sort_by_key(|(seq, _)| *seq);
        self.poke = false;
        Batch {
            ops: ops.into_iter().map(|(_, op)| op).collect(),
            overflow: std::mem::take(&mut self.overflow),
            viewing: self.viewing.take(),
            stop: self.stop,
            last: self.next,
        }
    }
}

/// What the handles and the writer share.
#[derive(Default)]
struct Shared {
    work: Mutex<Work>,
    /// The writer waits on it: every op queued, a poke and the close wake it.
    wake: Condvar,
    /// The writer has applied a batch and read the inbox after it (`Work::done`).
    applied: Condvar,
}

impl Shared {
    fn work(&self) -> MutexGuard<'_, Work> {
        self.work.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How a posted notice is kept.
enum Keeping {
    Toast,
    Nothing,
    State(KeyBuf),
    Event,
}

pub struct Inbox {
    shared: Arc<Shared>,
    events: Arc<Bus<InboxEvent>>,
    /// The subscription taken when the inbox started, for the first subscriber: the notices of the app's opening reach it.
    first: Mutex<Option<Subscription<InboxEvent>>>,
    listing: Arc<Mutex<Arc<Listing>>>,
    /// When each kept notice was last logged, by its key; only the keys logged within [`LOG_AGAIN`] are kept.
    logged: Mutex<BTreeMap<String, Instant>>,
    /// The thumbnails' ledger may hold files: a thumbnail the window makes is sent to clear its file only then.
    thumb_failures: Arc<AtomicBool>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Inbox {
    /// Start the inbox of the catalog at `catalog_path` (prepared): its writer opens its own connection. An error if the
    /// thread can't start.
    pub(crate) fn start(catalog_path: PathBuf) -> Result<Arc<Inbox>> {
        let shared = Arc::new(Shared::default());
        let events: Arc<Bus<InboxEvent>> = Arc::new(Bus::new(256));
        let first = Mutex::new(Some(events.subscribe()));
        let listing = Arc::new(Mutex::new(Arc::new(Listing::default())));
        let thumb_failures = Arc::new(AtomicBool::new(true));
        let writer = Writer::new(shared.clone(), events.clone(), listing.clone(), thumb_failures.clone());
        let thread = std::thread::Builder::new()
            .name("inbox".into())
            .spawn(move || writer.run(&catalog_path))
            .map_err(|e| failed(format!("can't start the inbox: {e}")))?;
        Ok(Arc::new(Inbox {
            shared,
            events,
            first,
            listing,
            logged: Mutex::new(BTreeMap::new()),
            thumb_failures,
            writer: Mutex::new(Some(thread)),
        }))
    }

    /// Queue work for the writer and wake it; `None` once the inbox is closed (the work is dropped: a post was only
    /// logged).
    ///
    /// Lock order: a caller may hold the library's writer slot (`Library::tend` posts write-stopped under it), so the
    /// order is writer slot → `Work`. Nothing else is locked while `Work` is held, and the inbox's writer never takes
    /// the slot.
    fn push<R>(&self, f: impl FnOnce(&mut Work) -> R) -> Option<R> {
        let mut work = self.shared.work();
        if work.stop {
            return None;
        }
        let result = f(&mut work);
        drop(work);
        self.shared.wake.notify_one();
        Some(result)
    }

    /// Tell the user `notice`: it is logged now, and kept as its lifecycle says (a transient one is only toasted).
    pub fn post(&self, notice: Notice) {
        self.log(&notice);
        let keeping = match notice.lifecycle() {
            Lifecycle::Transient => Keeping::Toast,
            // Built from the failure ledger when the inbox is read, never posted.
            Lifecycle::Ledger(_) => Keeping::Nothing,
            Lifecycle::State(key, _) => Keeping::State(key.to_buf()),
            Lifecycle::Event => Keeping::Event,
        };
        let at = now();
        match keeping {
            Keeping::Toast => self.events.publish(InboxEvent::Toast(notice)),
            Keeping::Nothing => {}
            Keeping::State(key) => {
                self.push(|w| w.state(StateOp::Post { key, notice, repeats: 1, at }));
            }
            Keeping::Event => {
                if self.push(|w| w.event(notice, at)) == Some(1) {
                    pg_log::warn("inbox", "the inbox's queue is full: notices are not kept");
                }
            }
        }
    }

    /// The kept notices of `scope` are now `notices` (`pg_catalog::Catalog::replace_notices`); none is a clear.
    pub fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
        if notices.is_empty() {
            self.clear(scope);
            return;
        }
        for notice in &notices {
            self.log(notice);
        }
        let at = now();
        self.push(|w| w.state(StateOp::Replace { scope, notices, at }));
    }

    /// The kept notices `key` names no longer hold (a key with no part names every part). Whether any is stored is
    /// the writer's to find (one indexed read): a clear of what isn't there takes no write lock.
    pub fn clear(&self, key: KeyBuf) {
        self.push(|w| w.state(StateOp::Clear { scope: key }));
    }

    /// Mark the entries `ids` read, as they were displayed (a row at its revision, a ledger up to its serial).
    pub fn mark_read(&self, ids: Vec<EntryId>) {
        if !ids.is_empty() {
            self.push(|w| w.mutation(Mutation::MarkRead(ids)));
        }
    }

    /// "Mark all read": the entries displayed, `ids`, and never one the window wasn't given.
    pub fn mark_all_read(&self, ids: Vec<EntryId>) {
        self.mark_read(ids);
    }

    /// Dismiss the entry `id`, as it was displayed.
    pub fn dismiss(&self, id: EntryId) {
        self.dismiss_all(vec![id]);
    }

    /// "Clear all": dismiss the entries displayed, `ids`, and never one the window wasn't given.
    pub fn dismiss_all(&self, ids: Vec<EntryId>) {
        if !ids.is_empty() {
            self.push(|w| w.mutation(Mutation::Dismiss(ids)));
        }
    }

    /// Try `pass`'s failed files again (`pg_catalog::Catalog::retry_failures`), one write of the writer's; `then` runs
    /// once that is written (the job is woken to make them). When it fails the inbox is read again, so the entry is shown
    /// as it is.
    pub fn retry(&self, pass: FilePass, then: Box<dyn FnOnce() + Send>) {
        self.push(|w| w.mutation(Mutation::Retry(pass, then)));
    }

    /// A thumbnail the window asked for couldn't be made: the thumbnails' ledger records it.
    pub fn thumb_failed(&self, failure: Failure) {
        self.thumb_failures.store(true, Ordering::Relaxed);
        self.push(|w| w.thumb(failure.file_id, Thumb::Failed(failure)));
    }

    /// A thumbnail the window asked for was made: the file leaves the thumbnails' ledger, if it is there. Nothing is sent
    /// while the ledger is empty.
    pub fn thumb_made(&self, file_id: FileId) {
        if self.thumb_failures.load(Ordering::Relaxed) {
            self.push(|w| w.thumb(file_id, Thumb::Made));
        }
    }

    /// The window shows the inbox now, or no longer: the ledgers' counts are read only while it does (the latest call
    /// wins). A change reads the inbox again.
    pub fn set_viewing(&self, viewing: bool) {
        self.push(|w| w.viewing = Some(viewing));
    }

    /// Look for what another process changed now, not at the next [`POLL`] (the window was activated, a pass wrote its
    /// ledger). Pokes coalesce.
    pub fn poke(&self) {
        self.push(|w| w.poke = true);
    }

    /// What the inbox lists now (empty until the writer has read it).
    pub fn listing(&self) -> Arc<Listing> {
        self.listing.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// What the inbox tells, from now on; the first call gets it from when the inbox started, so the notices of the app's
    /// opening are toasted.
    pub fn subscribe(&self) -> Subscription<InboxEvent> {
        self.first.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_else(|| self.events.subscribe())
    }

    /// Write what is queued (trying a busy catalog for at most [`SHUTDOWN_DRAIN`]) and stop the writer; later posts are
    /// only logged. Called at shutdown, before the catalog's connections close. Idempotent.
    pub(crate) fn close(&self) {
        self.shared.work().stop = true;
        self.shared.wake.notify_one();
        if let Some(writer) = self.writer.lock().unwrap_or_else(PoisonError::into_inner).take()
            && writer.join().is_err()
        {
            pg_log::error("inbox", "the inbox's writer panicked");
        }
    }

    /// Whether the writer has nothing left to do: nothing queued, and what it took is written and the inbox read after
    /// it. What the screenshot's settle check waits for (the inbox's marks and its listing are what it shows).
    pub fn idle(&self) -> bool {
        let work = self.shared.work();
        !work.has_work() && work.done >= work.next
    }

    /// Wait until the writer has applied everything queued before this call, and read the inbox after it if that was
    /// due; false after 20 s.
    #[cfg(test)]
    pub(crate) fn settle(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut work = self.shared.work();
        // A poke of its own, so there is a batch to wait for even when nothing is queued.
        let target = work.seq();
        work.poke = true;
        self.shared.wake.notify_one();
        while work.done < target {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            work = self.shared.applied.wait_timeout(work, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        true
    }

    /// Log `notice` as the developer reads it (its stored form); a kept one posted again soon after is not logged again.
    fn log(&self, notice: &Notice) {
        if let Lifecycle::State(key, _) = notice.lifecycle() {
            let key = format!("{key:?}");
            let mut logged = self.logged.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            if logged.get(&key).is_some_and(|at| now.duration_since(*at) < LOG_AGAIN) {
                return;
            }
            // A key not logged within LOG_AGAIN would be logged anyway: it goes, so the map holds the recent keys only.
            logged.retain(|_, at| now.duration_since(*at) < LOG_AGAIN);
            logged.insert(key, now);
        }
        write_log(notice);
    }
}

/// The library's job and writer post into the inbox. Every call only queues (see `Inbox::push` for the lock order).
impl NoticeSink for Inbox {
    fn post(&self, notice: Notice) {
        Inbox::post(self, notice);
    }

    fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
        Inbox::replace(self, scope, notices);
    }

    fn clear(&self, key: KeyBuf) {
        Inbox::clear(self, key);
    }

    fn poke(&self) {
        Inbox::poke(self);
    }
}

/// Log `notice` in its stored form, at its level.
fn write_log(notice: &Notice) {
    let text = serde_json::to_string(notice).unwrap_or_else(|e| format!("{notice:?} ({e})"));
    let target = notice.source().code();
    match notice.level() {
        Level::Info => pg_log::info(target, &text),
        Level::Warning => pg_log::warn(target, &text),
        Level::Error => pg_log::error(target, &text),
    }
}

/// What the writer writes and reads through: the catalog, or a test's store that fails on purpose.
trait Store {
    /// Post `notice` `times` times: one upsert counting them all.
    fn post(&self, notice: &Notice, times: u64, at: i64) -> pg_catalog::Result<bool>;
    fn replace(&self, scope: &Key<'_>, notices: &[Notice], at: i64) -> pg_catalog::Result<bool>;
    fn clear(&self, scope: &Key<'_>) -> pg_catalog::Result<bool>;
    fn mark(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool>;
    fn dismiss(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool>;
    fn retry(&self, pass: FilePass) -> pg_catalog::Result<()>;
    fn thumbs_failed(&self, failures: &[Failure], now: i64) -> pg_catalog::Result<bool>;
    fn thumbs_made(&self, files: &[FileId]) -> pg_catalog::Result<bool>;
    fn marks(&self) -> pg_catalog::Result<Vec<Mark>>;
    fn entries(&self, viewing: bool) -> pg_catalog::Result<Vec<Entry>>;
    fn thumb_failures(&self) -> pg_catalog::Result<bool>;
    fn version(&self) -> pg_catalog::Result<i64>;
    fn prune(&self, now: i64) -> pg_catalog::Result<usize>;

    /// Whether `e` is worth trying again: another connection held the lock past the busy timeout.
    fn is_busy(&self, e: &pg_catalog::Error) -> bool {
        e.is_busy()
    }
}

impl Store for Catalog {
    fn post(&self, notice: &Notice, times: u64, at: i64) -> pg_catalog::Result<bool> {
        self.post_notice_times(notice, times, at)
    }

    fn replace(&self, scope: &Key<'_>, notices: &[Notice], at: i64) -> pg_catalog::Result<bool> {
        self.replace_notices(scope, notices, at)
    }

    fn clear(&self, scope: &Key<'_>) -> pg_catalog::Result<bool> {
        self.clear_notices(scope)
    }

    fn mark(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool> {
        self.mark_all_read(ids, now)
    }

    fn dismiss(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool> {
        self.dismiss_all(ids, now)
    }

    fn retry(&self, pass: FilePass) -> pg_catalog::Result<()> {
        self.retry_failures(pass)
    }

    fn thumbs_failed(&self, failures: &[Failure], now: i64) -> pg_catalog::Result<bool> {
        self.record_failures(FilePass::Thumbs, failures, now)
    }

    fn thumbs_made(&self, files: &[FileId]) -> pg_catalog::Result<bool> {
        self.clear_failures(FilePass::Thumbs, files)
    }

    fn marks(&self) -> pg_catalog::Result<Vec<Mark>> {
        self.inbox_marks()
    }

    fn entries(&self, viewing: bool) -> pg_catalog::Result<Vec<Entry>> {
        self.inbox_entries(LISTED, viewing)
    }

    fn thumb_failures(&self) -> pg_catalog::Result<bool> {
        self.any_failures(FilePass::Thumbs)
    }

    fn version(&self) -> pg_catalog::Result<i64> {
        self.data_version()
    }

    fn prune(&self, now: i64) -> pg_catalog::Result<usize> {
        self.prune_inbox(now)
    }
}

/// One write of the writer's: an op, or a run of thumbnail reports of one kind written together.
enum Step {
    Event { notice: Notice, at: i64 },
    State(StateOp),
    Mutation(Mutation),
    ThumbsFailed(Vec<Failure>),
    ThumbsMade(Vec<FileId>),
}

impl Step {
    /// How many notices are lost when it can't be written (a mark, a dismissal or a Retry is not a notice).
    fn notices(&self) -> u64 {
        match self {
            Step::Event { .. } | Step::State(StateOp::Clear { .. }) => 1,
            Step::State(StateOp::Post { repeats, .. }) => *repeats,
            Step::State(StateOp::Replace { notices, .. }) => count(notices.len()).max(1),
            Step::ThumbsFailed(failures) => count(failures.len()),
            Step::Mutation(_) | Step::ThumbsMade(_) => 0,
        }
    }
}

/// The ops as the writer writes them, in their order: each thumbnail report joins the run of its kind just before it.
fn steps(ops: Vec<Op>) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::Event { notice, at } => out.push(Step::Event { notice, at }),
            Op::State(op) => out.push(Step::State(op)),
            Op::Mutation(m) => out.push(Step::Mutation(m)),
            Op::Thumb(_, Thumb::Failed(failure)) => match out.last_mut() {
                Some(Step::ThumbsFailed(run)) => run.push(failure),
                _ => out.push(Step::ThumbsFailed(vec![failure])),
            },
            Op::Thumb(file, Thumb::Made) => match out.last_mut() {
                Some(Step::ThumbsMade(run)) => run.push(file),
                _ => out.push(Step::ThumbsMade(vec![file])),
            },
        }
    }
    out
}

/// Write `step`. It is one write each, so a try again after a busy catalog writes it once. Whether the inbox changed.
fn apply(store: &impl Store, step: &mut Step, now: i64) -> pg_catalog::Result<bool> {
    match step {
        Step::Event { notice, at } => store.post(notice, 1, *at),
        // One upsert for the folded posts: the count rises by all of them.
        Step::State(StateOp::Post { notice, repeats, at, .. }) => store.post(notice, *repeats, *at),
        Step::State(StateOp::Replace { scope, notices, at }) => store.replace(&scope.key(), notices, *at),
        Step::State(StateOp::Clear { scope }) => store.clear(&scope.key()),
        Step::Mutation(Mutation::MarkRead(ids)) => store.mark(ids, now),
        Step::Mutation(Mutation::Dismiss(ids)) => store.dismiss(ids, now),
        Step::Mutation(Mutation::Retry(pass, _)) => store.retry(*pass).map(|()| true),
        Step::ThumbsFailed(failures) => store.thumbs_failed(failures, now),
        Step::ThumbsMade(files) => store.thumbs_made(files),
    }
}

/// What follows a step once it is written: a Retry's `then`.
fn finish(step: Step) {
    if let Step::Mutation(Mutation::Retry(_, then)) = step {
        then();
    }
}

/// Why a step wasn't written.
enum Unwritten {
    /// It failed, not busy: it is not tried again.
    Failed(pg_catalog::Error),
    /// The catalog stayed busy until the shutdown's deadline.
    Abandoned,
}

/// What a batch's writes came to.
#[derive(Default)]
struct Drained {
    /// A write changed the inbox.
    changed: bool,
    /// A write went through.
    committed: bool,
    /// A write failed: the inbox is read again (it undoes the window's optimistic change).
    force: bool,
}

/// The writer's thread: what it publishes to, and what it remembers between reads.
struct Writer {
    shared: Arc<Shared>,
    events: Arc<Bus<InboxEvent>>,
    listing: Arc<Mutex<Arc<Listing>>>,
    thumb_failures: Arc<AtomicBool>,
    /// What the last read found, by entry (`EntryId::entry`).
    seen: BTreeMap<EntryId, Mark>,
    /// When each entry was last toasted, within [`LOG_AGAIN`]: a state's new revision toasts at most once in it.
    toasted: BTreeMap<EntryId, Instant>,
    /// The kinds whose stored rows couldn't be read, already logged: once per kind per process.
    undecodable: BTreeSet<String>,
    /// Whether the inbox is viewed: the ledgers' counts are read only then.
    viewing: bool,
    /// Notices not kept since the last [`Notice::NoticesLost`].
    lost: u64,
    /// The catalog's `data_version` at the last read: it moves when another connection commits.
    version: Option<i64>,
}

impl Writer {
    fn new(
        shared: Arc<Shared>,
        events: Arc<Bus<InboxEvent>>,
        listing: Arc<Mutex<Arc<Listing>>>,
        thumb_failures: Arc<AtomicBool>,
    ) -> Writer {
        Writer {
            shared,
            events,
            listing,
            thumb_failures,
            seen: BTreeMap::new(),
            toasted: BTreeMap::new(),
            undecodable: BTreeSet::new(),
            viewing: false,
            lost: 0,
            version: None,
        }
    }

    fn run(mut self, catalog_path: &Path) {
        let catalog = match Catalog::open(catalog_path, pg_platform::native::PATH_RULES) {
            Ok(catalog) => catalog,
            Err(e) => {
                pg_log::error("inbox", &format!("the inbox can't open the catalog, so nothing is kept: {e:#}"));
                // Nothing will take the work: later posts are only logged.
                self.shared.work().stop = true;
                return;
            }
        };
        if let Err(e) = catalog.set_busy_timeout(BUSY_TIMEOUT) {
            pg_log::warn("inbox", &format!("the inbox's busy timeout couldn't be set: {e:#}"));
        }
        self.serve(&catalog);
    }

    /// Read the inbox (what it held when the app started is not news: listed, not toasted), then write what is queued
    /// until the inbox closes.
    fn serve(&mut self, store: &impl Store) {
        if let Err(e) = store.prune(now()) {
            pg_log::warn("inbox", &format!("the inbox couldn't be pruned: {e:#}"));
        }
        self.refresh(store, false);
        self.version = store.version().ok();
        loop {
            let batch = self.next_batch();
            if !self.handle(store, batch) {
                return;
            }
        }
    }

    /// The next batch: once there is work, or the close, or after [`POLL`] (to look at `data_version`).
    fn next_batch(&self) -> Batch {
        let until = Instant::now() + POLL;
        let mut work = self.shared.work();
        while !work.has_work() && !work.stop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            work = self.shared.wake.wait_timeout(work, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        work.take()
    }

    /// Write `batch`, say what was lost, and read the inbox again when it may have changed. False once it was the last
    /// (the inbox closed).
    fn handle(&mut self, store: &impl Store, batch: Batch) -> bool {
        let mut deadline = batch.stop.then(|| Instant::now() + SHUTDOWN_DRAIN);
        self.lost = self.lost.saturating_add(batch.overflow);
        let drained = self.drain(store, batch.ops, &mut deadline);
        let reported = self.report_lost(store, drained.committed);
        if batch.stop {
            self.done(batch.last);
            return false;
        }
        let viewed = batch.viewing.is_some_and(|v| v != self.viewing);
        if let Some(viewing) = batch.viewing {
            self.viewing = viewing;
        }
        // Read outside a transaction: it moves only when another connection has committed.
        let version = store.version().ok();
        if drained.changed || drained.force || reported || viewed || version != self.version {
            self.version = version;
            self.refresh(store, true);
        }
        self.done(batch.last);
        true
    }

    /// Write the ops in their order.
    fn drain(&mut self, store: &impl Store, ops: Vec<Op>, deadline: &mut Option<Instant>) -> Drained {
        let mut out = Drained::default();
        let mut steps = steps(ops).into_iter();
        while let Some(mut step) = steps.next() {
            match self.write(store, &mut step, deadline) {
                Ok(changed) => {
                    out.committed = true;
                    out.changed |= changed;
                    finish(step);
                }
                Err(Unwritten::Failed(e)) => {
                    out.force = true;
                    let notices = step.notices();
                    if notices > 0 {
                        self.lost = self.lost.saturating_add(notices);
                        pg_log::warn("inbox", &format!("the inbox couldn't keep {notices} notices: {e:#}"));
                    } else {
                        pg_log::warn("inbox", &format!("the inbox couldn't be changed: {e:#}"));
                    }
                }
                Err(Unwritten::Abandoned) => {
                    let rest: Vec<Step> = std::iter::once(step).chain(steps.by_ref()).collect();
                    let notices: u64 = rest.iter().map(Step::notices).sum();
                    let changes = rest.iter().filter(|s| matches!(s, Step::Mutation(_))).count();
                    pg_log::warn(
                        "inbox",
                        &format!(
                            "the inbox closed with the catalog still busy: {notices} notices and {changes} of the \
                             user's changes weren't written"
                        ),
                    );
                    break;
                }
            }
        }
        out
    }

    /// Write `step`, trying again while the catalog is busy (until the shutdown's deadline, once the inbox closes).
    fn write(
        &self,
        store: &impl Store,
        step: &mut Step,
        deadline: &mut Option<Instant>,
    ) -> std::result::Result<bool, Unwritten> {
        let mut tries = 0u32;
        loop {
            match apply(store, step, now()) {
                Ok(changed) => return Ok(changed),
                Err(e) if store.is_busy(&e) => {
                    tries = tries.saturating_add(1);
                    if tries == BUSY_WARN {
                        pg_log::warn("inbox", &format!("the catalog stays busy: the inbox waits for it ({e:#})"));
                    }
                    if !self.pause(deadline) {
                        return Err(Unwritten::Abandoned);
                    }
                }
                Err(e) => return Err(Unwritten::Failed(e)),
            }
        }
    }

    /// Wait a little before a busy write is tried again: false once the shutdown's deadline has passed. The close sets
    /// the deadline and wakes the wait.
    fn pause(&self, deadline: &mut Option<Instant>) -> bool {
        let work = self.shared.work();
        if work.stop && deadline.is_none() {
            *deadline = Some(Instant::now() + SHUTDOWN_DRAIN);
        }
        let wait = match *deadline {
            Some(at) => match at.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => left.min(RETRY_PAUSE),
                _ => return false,
            },
            None => RETRY_PAUSE,
        };
        drop(self.shared.wake.wait_timeout(work, wait).unwrap_or_else(PoisonError::into_inner));
        true
    }

    /// After a write that went through, say how many notices weren't kept since the last time; whether it was said.
    fn report_lost(&mut self, store: &impl Store, committed: bool) -> bool {
        if self.lost == 0 || !committed {
            return false;
        }
        let notice = Notice::NoticesLost { count: self.lost };
        write_log(&notice);
        match store.post(&notice, 1, now()) {
            Ok(_) => {
                self.lost = 0;
                true
            }
            Err(e) => {
                pg_log::warn("inbox", &format!("the inbox couldn't say what it lost: {e:#}"));
                false
            }
        }
    }

    fn done(&self, last: u64) {
        self.shared.work().done = last;
        self.shared.applied.notify_all();
    }

    /// Read the inbox and publish what it lists; with `toast`, each entry that is news is toasted (the toast rule, in
    /// the module's header).
    fn refresh(&mut self, store: &impl Store, toast: bool) {
        let read = store.marks().and_then(|marks| Ok((marks, store.entries(self.viewing)?)));
        let (marks, entries) = match read {
            Ok(read) => read,
            Err(e) => {
                pg_log::warn("inbox", &format!("the inbox couldn't be read: {e:#}"));
                return;
            }
        };
        for entry in &entries {
            if let (Some(error), Notice::Unknown { code }) = (&entry.decode_error, &entry.notice)
                && self.undecodable.insert(code.clone())
            {
                pg_log::warn("inbox", &format!("a stored {code} notice couldn't be read: {error}"));
            }
        }
        let now = Instant::now();
        self.toasted.retain(|_, at| now.duration_since(*at) < LOG_AGAIN);
        let mut news = BTreeSet::new();
        for mark in marks.iter().filter(|m| !m.read) {
            let entry = mark.id.entry();
            let is_news = match self.seen.get(&entry) {
                // New, or unread again.
                None => true,
                Some(before) if before.read => true,
                // A state's new revision while it is unread, once per LOG_AGAIN.
                Some(before) => revised(before.id, mark.id) && !self.toasted.contains_key(&entry),
            };
            if is_news {
                news.insert(entry);
            }
        }
        let unread = count(marks.iter().filter(|m| !m.read).count());
        self.seen = marks.into_iter().map(|m| (m.id.entry(), m)).collect();
        match store.thumb_failures() {
            Ok(any) => self.thumb_failures.store(any, Ordering::Relaxed),
            Err(e) => pg_log::warn("inbox", &format!("the thumbnails' ledger couldn't be read: {e:#}")),
        }
        let listing = Arc::new(Listing { entries, unread });
        *self.listing.lock().unwrap_or_else(PoisonError::into_inner) = listing.clone();
        if toast {
            for entry in listing.entries.iter().filter(|e| news.contains(&e.id.entry())) {
                self.toasted.insert(entry.id.entry(), now);
                // A ledger's entry is never posted: it is logged when it is news.
                if let EntryId::Ledger { .. } = entry.id {
                    let text = serde_json::to_string(&entry.notice).unwrap_or_else(|e| format!("{e}"));
                    let text = match entry.counts {
                        Some(counts) => format!("{text} ({} files)", counts.total),
                        None => text,
                    };
                    pg_log::warn(entry.notice.source().code(), &text);
                }
                self.events.publish(InboxEvent::Toast(entry.notice.clone()));
            }
        }
        self.events.publish(InboxEvent::Changed(listing));
    }
}

/// Whether a row's revision rose from `before` to `now` (a state that says something new).
fn revised(before: EntryId, now: EntryId) -> bool {
    matches!((before, now), (EntryId::Row { rev: was, .. }, EntryId::Row { rev: is, .. }) if is > was)
}

/// A count as the inbox holds it.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Now, in Unix seconds (the catalog's times).
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_catalog::fixture::{self, File};
    use pg_model::ids::FolderId;
    use pg_model::notice::{EditOp, LedgerKind, Part};
    use pg_model::turns::Turns;
    use std::sync::atomic::AtomicUsize;

    /// A prepared catalog in a test folder, and its path.
    fn catalog_in(name: &str) -> (PathBuf, PathBuf) {
        let dir = pg_testkit::run_folder("scratch_app", name).unwrap();
        let layout = pg_library::layout::Layout::in_home(&dir);
        std::fs::create_dir_all(layout.catalog_dir()).unwrap();
        drop(pg_library::open_catalog(&layout).unwrap());
        (layout.catalog_path(), dir)
    }

    fn open(path: &Path) -> Catalog {
        Catalog::open(path, pg_platform::native::PATH_RULES).unwrap()
    }

    fn inbox(name: &str) -> (Arc<Inbox>, PathBuf, PathBuf) {
        let (path, dir) = catalog_in(name);
        (Inbox::start(path.clone()).unwrap(), path, dir)
    }

    /// The next listing the inbox publishes, and the toasts before it.
    fn next(sub: &Subscription<InboxEvent>) -> (Vec<Notice>, Arc<Listing>) {
        let mut toasts = Vec::new();
        loop {
            match sub.recv().expect("the inbox is open") {
                InboxEvent::Toast(notice) => toasts.push(notice),
                InboxEvent::Changed(listing) => return (toasts, listing),
            }
        }
    }

    fn failed(error: &str) -> Notice {
        Notice::EditFailed { op: EditOp::Rotate, error: error.into() }
    }

    fn event(i: u64) -> Notice {
        Notice::RescanFailed { error: format!("event {i}") }
    }

    fn total(entry: &Entry) -> Option<u64> {
        entry.counts.map(|c| c.total)
    }

    fn notices(listing: &Listing) -> Vec<Notice> {
        listing.entries.iter().map(|e| e.notice.clone()).collect()
    }

    /// A file the catalog at `path` holds, in a folder of the test's own, and its failure as the fixture made it.
    fn a_file(path: &Path, dir: &Path) -> Failure {
        let c = open(path);
        let folder = dir.to_str().unwrap();
        fixture::state::add_scan(&c, 1, folder).unwrap();
        fixture::state::add_folder(&c, 1, folder).unwrap();
        fixture::insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 0)]).unwrap();
        Failure { file_id: FileId::new(1), mtime: 0, size: 10, rotation: Turns::NONE, error: "x".into(), retry: false }
    }

    /// A writer on its own `Work`, as the inbox's thread has it, and a subscription to what it publishes.
    fn writer() -> (Writer, Subscription<InboxEvent>) {
        let events: Arc<Bus<InboxEvent>> = Arc::new(Bus::new(1024));
        let sub = events.subscribe();
        let writer = Writer::new(
            Arc::new(Shared::default()),
            events,
            Arc::new(Mutex::new(Arc::new(Listing::default()))),
            Arc::new(AtomicBool::new(true)),
        );
        (writer, sub)
    }

    fn toasts(sub: &Subscription<InboxEvent>) -> Vec<Notice> {
        sub.take()
            .items
            .into_iter()
            .filter_map(|e| match e {
                InboxEvent::Toast(n) => Some(n),
                InboxEvent::Changed(_) => None,
            })
            .collect()
    }

    fn batch(ops: Vec<Op>) -> Batch {
        Batch { ops, overflow: 0, viewing: None, stop: false, last: 0 }
    }

    fn post_op(notice: Notice) -> StateOp {
        let Lifecycle::State(key, _) = notice.lifecycle() else { panic!("a state") };
        StateOp::Post { key: key.to_buf(), notice, repeats: 1, at: now() }
    }

    /// A fault a test's store raises.
    #[derive(Debug, PartialEq, Eq, thiserror::Error)]
    enum Injected {
        #[error("busy (injected)")]
        Busy,
        #[error("locked (injected)")]
        Locked,
    }

    /// The catalog, with faults raised by its writes, in order, and its writes counted.
    struct Faulty {
        catalog: Catalog,
        faults: Mutex<VecDeque<Injected>>,
        writes: AtomicUsize,
    }

    impl Faulty {
        fn new(catalog: Catalog, faults: impl IntoIterator<Item = Injected>) -> Faulty {
            Faulty { catalog, faults: Mutex::new(faults.into_iter().collect()), writes: AtomicUsize::new(0) }
        }

        fn write<T>(&self, f: impl FnOnce(&Catalog) -> pg_catalog::Result<T>) -> pg_catalog::Result<T> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            match self.faults.lock().unwrap().pop_front() {
                Some(fault) => Err(pg_catalog::Error::Other(anyhow::Error::new(fault))),
                None => f(&self.catalog),
            }
        }
    }

    impl Store for Faulty {
        fn post(&self, notice: &Notice, times: u64, at: i64) -> pg_catalog::Result<bool> {
            self.write(|c| c.post_notice_times(notice, times, at))
        }
        fn replace(&self, scope: &Key<'_>, notices: &[Notice], at: i64) -> pg_catalog::Result<bool> {
            self.write(|c| c.replace_notices(scope, notices, at))
        }
        fn clear(&self, scope: &Key<'_>) -> pg_catalog::Result<bool> {
            self.write(|c| c.clear_notices(scope))
        }
        fn mark(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool> {
            self.write(|c| c.mark_all_read(ids, now))
        }
        fn dismiss(&self, ids: &[EntryId], now: i64) -> pg_catalog::Result<bool> {
            self.write(|c| c.dismiss_all(ids, now))
        }
        fn retry(&self, pass: FilePass) -> pg_catalog::Result<()> {
            self.write(|c| c.retry_failures(pass))
        }
        fn thumbs_failed(&self, failures: &[Failure], now: i64) -> pg_catalog::Result<bool> {
            self.write(|c| c.record_failures(FilePass::Thumbs, failures, now))
        }
        fn thumbs_made(&self, files: &[FileId]) -> pg_catalog::Result<bool> {
            self.write(|c| c.clear_failures(FilePass::Thumbs, files))
        }
        fn marks(&self) -> pg_catalog::Result<Vec<Mark>> {
            self.catalog.inbox_marks()
        }
        fn entries(&self, viewing: bool) -> pg_catalog::Result<Vec<Entry>> {
            self.catalog.inbox_entries(LISTED, viewing)
        }
        fn thumb_failures(&self) -> pg_catalog::Result<bool> {
            self.catalog.any_failures(FilePass::Thumbs)
        }
        fn version(&self) -> pg_catalog::Result<i64> {
            self.catalog.data_version()
        }
        fn prune(&self, now: i64) -> pg_catalog::Result<usize> {
            self.catalog.prune_inbox(now)
        }
        fn is_busy(&self, e: &pg_catalog::Error) -> bool {
            matches!(e, pg_catalog::Error::Other(a) if a.downcast_ref::<Injected>() == Some(&Injected::Busy))
                || e.is_busy()
        }
    }

    // --- The work log, alone.

    // Every op takes one sequence, and the writer gets them in it across the kinds: a thumbnail's failure reported
    // before a Retry comes before it, whatever kind each is.
    #[test]
    fn the_ops_come_in_one_sequence_across_kinds() {
        let mut work = Work::default();
        let x = Failure {
            file_id: FileId::new(7),
            mtime: 0,
            size: 1,
            rotation: Turns::NONE,
            error: "x".into(),
            retry: false,
        };
        work.thumb(x.file_id, Thumb::Failed(x));
        work.mutation(Mutation::Retry(FilePass::Thumbs, Box::new(|| {})));
        work.event(event(1), 0);
        work.state(post_op(failed("a")));
        let kinds: Vec<&str> = work
            .take()
            .ops
            .iter()
            .map(|op| match op {
                Op::Thumb(..) => "thumb",
                Op::Mutation(_) => "mutation",
                Op::Event { .. } => "event",
                Op::State(_) => "state",
            })
            .collect();
        assert_eq!(kinds, ["thumb", "mutation", "event", "state"]);
    }

    // A post then a clear of every part of its kind: only the clear is left. A clear then a post: both, in order.
    #[test]
    fn a_clear_supersedes_what_it_covers_and_nothing_after_it() {
        let mut work = Work::default();
        work.state(post_op(failed("a")));
        let all = Key::all("edit-failed", None).to_buf();
        work.state(StateOp::Replace { scope: all.clone(), notices: vec![failed("b")], at: 0 });
        work.state(StateOp::Clear { scope: all.clone() });
        // Another kind is not covered.
        work.state(post_op(Notice::PrefsNotSaved { path: "p".into(), error: "p".into() }));
        work.state(post_op(failed("c")));
        let ops = work.take().ops;
        let shape: Vec<String> = ops
            .iter()
            .map(|op| match op {
                Op::State(StateOp::Clear { scope }) => format!("clear {}", scope.code),
                Op::State(StateOp::Post { key, .. }) => format!("post {}", key.code),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(shape, ["clear edit-failed", "post prefs-not-saved", "post edit-failed"]);
        // A clear of one part leaves the others, and a scope with every part covers a narrower one.
        let mut work = Work::default();
        let part = |op: EditOp| KeyBuf { code: "edit-failed", root: None, part: Some(Part::Code(op.code())) };
        work.state(StateOp::Post {
            key: part(EditOp::Rate),
            notice: Notice::EditFailed { op: EditOp::Rate, error: "r".into() },
            repeats: 1,
            at: 0,
        });
        work.state(StateOp::Clear { scope: part(EditOp::Rotate) });
        assert_eq!(work.states.len(), 2);
        work.state(StateOp::Clear { scope: all });
        assert_eq!(work.states.len(), 1);
    }

    // Posts of one key between two takes are one, the latest, with every repeat counted, whatever each said.
    #[test]
    fn posts_of_a_key_are_one_with_their_repeats_summed() {
        let mut work = Work::default();
        for error in ["a", "b", "c"] {
            work.state(post_op(failed(error)));
        }
        let ops = work.take().ops;
        let [Op::State(StateOp::Post { notice, repeats, .. })] = &ops[..] else { panic!("one post") };
        assert_eq!((notice, *repeats), (&failed("c"), 3));
    }

    // A full event queue counts what it has no room for, and a state op, a mutation, a thumbnail report and the stop
    // all still get through.
    #[test]
    fn a_full_event_queue_loses_events_only() {
        let mut work = Work::default();
        for i in 0..count(EVENTS) {
            assert_eq!(work.event(event(i), 0), 0);
        }
        assert_eq!(work.event(event(9999), 0), 1);
        assert_eq!(work.event(event(9999), 0), 2);
        work.state(post_op(failed("a")));
        work.mutation(Mutation::MarkRead(vec![EntryId::Row { id: 1, uid: 1, rev: 0 }]));
        work.thumb(FileId::new(1), Thumb::Made);
        work.stop = true;
        let batch = work.take();
        assert_eq!((batch.ops.len(), batch.overflow, batch.stop), (EVENTS + 3, 2, true));
        assert_eq!(work.take().overflow, 0, "counted once");
    }

    // Whether the inbox is viewed: the latest wins, and it is taken once.
    #[test]
    fn viewing_is_the_latest() {
        let mut work = Work { viewing: Some(true), ..Work::default() };
        work.viewing = Some(false);
        assert!(work.has_work());
        assert_eq!(work.take().viewing, Some(false));
        assert_eq!(work.take().viewing, None);
    }

    // A file's thumbnail reports are its latest only, and the runs of one kind are written together.
    #[test]
    fn a_files_thumbnail_reports_are_its_latest() {
        let mut work = Work::default();
        let fail = |id| Failure {
            file_id: FileId::new(id),
            mtime: 0,
            size: 1,
            rotation: Turns::NONE,
            error: "x".into(),
            retry: false,
        };
        work.thumb(FileId::new(1), Thumb::Failed(fail(1)));
        work.thumb(FileId::new(2), Thumb::Failed(fail(2)));
        work.thumb(FileId::new(1), Thumb::Made);
        work.thumb(FileId::new(3), Thumb::Made);
        let steps = steps(work.take().ops);
        let shape: Vec<(char, usize)> = steps
            .iter()
            .map(|s| match s {
                Step::ThumbsFailed(f) => ('f', f.len()),
                Step::ThumbsMade(m) => ('m', m.len()),
                _ => ('?', 0),
            })
            .collect();
        assert_eq!(shape, [('f', 1), ('m', 2)]);
    }

    // --- The writer, through its store.

    // A thumbnail's failure then the user's Retry, in one batch: written in that order, so the Retry relists the file
    // and the ledger is empty after; the other way round, the failure stays.
    #[test]
    fn a_failure_then_a_retry_is_written_in_that_order() {
        let (path, dir) = catalog_in("inbox-sequence");
        let x = a_file(&path, &dir);
        let catalog = open(&path);
        let (mut writer, _sub) = writer();
        let woken = Arc::new(AtomicBool::new(false));
        let mut work = Work::default();
        work.thumb(x.file_id, Thumb::Failed(x.clone()));
        let then = woken.clone();
        work.mutation(Mutation::Retry(FilePass::Thumbs, Box::new(move || then.store(true, Ordering::Relaxed))));
        assert!(writer.handle(&catalog, work.take()));
        assert!(catalog.recorded_failures(FilePass::Thumbs).unwrap().is_empty());
        assert!(woken.load(Ordering::Relaxed), "the job is woken once the Retry is written");
        work.mutation(Mutation::Retry(FilePass::Thumbs, Box::new(|| {})));
        work.thumb(x.file_id, Thumb::Failed(x.clone()));
        writer.handle(&catalog, work.take());
        assert_eq!(catalog.recorded_failures(FilePass::Thumbs).unwrap().len(), 1);
        drop(catalog);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Folded posts are one row counted as many times as they were posted, holding the latest.
    #[test]
    fn folded_posts_count_every_repeat() {
        let (path, dir) = catalog_in("inbox-repeats");
        let catalog = open(&path);
        let (mut writer, _sub) = writer();
        let mut work = Work::default();
        for error in ["a", "b", "c"] {
            work.state(post_op(failed(error)));
        }
        writer.handle(&catalog, work.take());
        let entries = catalog.inbox_entries(10, false).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].notice.clone(), total(&entries[0])), (failed("c"), Some(3)));
        drop(catalog);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // SQLITE_LOCKED (and any failure but busy) is not tried again: a lost notice is counted, and said once after the
    // next write that goes through; a batch whose writes all fail says nothing.
    #[test]
    fn locked_is_not_tried_again_and_lost_notices_are_said_after_the_next_commit() {
        let (path, dir) = catalog_in("inbox-locked");
        let store = Faulty::new(open(&path), [Injected::Locked, Injected::Locked]);
        let (mut writer, _sub) = writer();
        let lost = Batch { overflow: 3, ..batch(vec![Op::Event { notice: event(1), at: 1 }]) };
        writer.handle(&store, lost);
        assert_eq!(store.writes.load(Ordering::Relaxed), 1, "not tried again");
        assert_eq!(writer.lost, 4, "the overflow and the failed event");
        writer.handle(&store, batch(vec![Op::Event { notice: event(2), at: 2 }]));
        assert_eq!(writer.lost, 5);
        assert!(store.catalog.inbox_entries(10, false).unwrap().is_empty(), "nothing went through: nothing said");
        writer.handle(&store, batch(vec![Op::Event { notice: event(3), at: 3 }]));
        assert_eq!(writer.lost, 0);
        let kept = notices(&writer.listing.lock().unwrap());
        assert_eq!(kept.iter().filter(|n| matches!(n, Notice::NoticesLost { count: 5 })).count(), 1, "{kept:?}");
        assert!(kept.contains(&event(3)));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A busy write is tried again, in order, until it goes through.
    #[test]
    fn busy_is_tried_again_in_order() {
        let (path, dir) = catalog_in("inbox-busy-seam");
        let store = Faulty::new(open(&path), [Injected::Busy, Injected::Busy]);
        let (mut writer, _sub) = writer();
        let ops = vec![Op::Event { notice: event(1), at: 1 }, Op::Event { notice: event(2), at: 1 }];
        writer.handle(&store, batch(ops));
        assert_eq!(store.writes.load(Ordering::Relaxed), 4);
        assert_eq!(writer.lost, 0);
        assert_eq!(notices(&writer.listing.lock().unwrap()), [event(2), event(1)]);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A dismissal that fails is logged, not counted as lost, and the inbox is read again (the window's optimistic
    // change is undone: the entry is listed again).
    #[test]
    fn a_failed_mutation_reads_the_inbox_again_and_is_not_lost() {
        let (path, dir) = catalog_in("inbox-mutation");
        let store = Faulty::new(open(&path), []);
        let (mut writer, sub) = writer();
        writer.handle(&store, batch(vec![Op::State(post_op(failed("a")))]));
        let id = writer.listing.lock().unwrap().entries[0].id;
        sub.take();
        store.faults.lock().unwrap().push_back(Injected::Locked);
        writer.handle(&store, batch(vec![Op::Mutation(Mutation::Dismiss(vec![id]))]));
        assert_eq!(writer.lost, 0);
        let published: Vec<Arc<Listing>> = sub
            .take()
            .items
            .into_iter()
            .filter_map(|e| match e {
                InboxEvent::Changed(l) => Some(l),
                InboxEvent::Toast(_) => None,
            })
            .collect();
        assert_eq!(published.len(), 1, "read again");
        assert_eq!(notices(&published[0]), [failed("a")]);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A failed Retry doesn't wake the job, and the inbox is read again.
    #[test]
    fn a_failed_retry_wakes_nothing() {
        let (path, dir) = catalog_in("inbox-retry-fails");
        let store = Faulty::new(open(&path), [Injected::Locked]);
        let (mut writer, sub) = writer();
        let woken = Arc::new(AtomicBool::new(false));
        let then = woken.clone();
        let retry = Mutation::Retry(FilePass::Thumbs, Box::new(move || then.store(true, Ordering::Relaxed)));
        writer.handle(&store, batch(vec![Op::Mutation(retry)]));
        assert!(!woken.load(Ordering::Relaxed));
        assert!(sub.take().items.iter().any(|e| matches!(e, InboxEvent::Changed(_))));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A state toasts when it is new; its next revision while unread is news once per LOG_AGAIN (the first toast
    // counts); read and posted again with new data, it toasts again.
    #[test]
    fn a_states_new_revision_toasts_once_per_log_again() {
        let (path, dir) = catalog_in("inbox-rev");
        let catalog = open(&path);
        let (mut writer, sub) = writer();
        writer.refresh(&catalog, false);
        let post = |w: &mut Writer, error: &str| w.handle(&catalog, batch(vec![Op::State(post_op(failed(error)))]));
        post(&mut writer, "a");
        assert_eq!(toasts(&sub), [failed("a")]);
        post(&mut writer, "b");
        assert!(toasts(&sub).is_empty(), "throttled: toasted within LOG_AGAIN");
        // As if LOG_AGAIN had passed.
        for at in writer.toasted.values_mut() {
            *at = Instant::now().checked_sub(LOG_AGAIN + Duration::from_secs(1)).unwrap();
        }
        post(&mut writer, "c");
        assert_eq!(toasts(&sub), [failed("c")]);
        // The same again: not news.
        post(&mut writer, "c");
        assert!(toasts(&sub).is_empty());
        // Read, then new data: unread again, which always toasts.
        let id = writer.listing.lock().unwrap().entries[0].id;
        writer.handle(&catalog, batch(vec![Op::Mutation(Mutation::MarkRead(vec![id]))]));
        post(&mut writer, "d");
        assert_eq!(toasts(&sub), [failed("d")]);
        drop(catalog);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- The inbox, with its thread.

    // A post is kept and toasted once; the same again is counted and not toasted; a mark and a clear take it out of
    // the listing. The first subscriber has every listing from the start, the baseline untoasted.
    #[test]
    fn a_post_is_listed_and_toasted_once() {
        let (inbox, _, dir) = inbox("inbox-post");
        let sub = inbox.subscribe();
        let (toasts, listing) = next(&sub);
        assert!(toasts.is_empty() && listing.entries.is_empty());
        inbox.post(failed("a"));
        let (toasts, listing) = next(&sub);
        assert_eq!(toasts, [failed("a")]);
        assert_eq!((listing.entries.len(), listing.unread), (1, 1));
        inbox.post(failed("a"));
        let (toasts, listing) = next(&sub);
        assert!(toasts.is_empty());
        assert_eq!(total(&listing.entries[0]), Some(2));
        inbox.mark_read(vec![listing.entries[0].id]);
        assert_eq!(next(&sub).1.unread, 0);
        inbox.clear(Key::all("edit-failed", None).to_buf());
        assert!(next(&sub).1.entries.is_empty());
        // A transient notice is only toasted.
        inbox.post(Notice::FileNoLongerInLibrary);
        assert!(matches!(sub.recv(), Some(InboxEvent::Toast(Notice::FileNoLongerInLibrary))));
        inbox.close();
        inbox.close();
        // After the close a post is only logged.
        inbox.post(failed("b"));
        assert_eq!(inbox.listing().entries.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // What another connection writes is found by a poke, and toasted.
    #[test]
    fn another_connections_post_is_found_by_a_poke() {
        let (inbox, path, dir) = inbox("inbox-poke");
        let sub = inbox.subscribe();
        next(&sub);
        let other = open(&path);
        other.post_notice(&failed("elsewhere"), now()).unwrap();
        inbox.poke();
        let (toasts, listing) = next(&sub);
        assert_eq!(toasts, [failed("elsewhere")]);
        assert_eq!(listing.unread, 1);
        drop(other);
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Another connection holding the write lock for a second (well past the writer's busy timeout): what was queued
    // meanwhile is written after it, in order.
    #[test]
    fn a_held_write_lock_delays_the_writes_in_order() {
        let (inbox, path, dir) = inbox("inbox-held");
        assert!(inbox.settle());
        let mut other = open(&path);
        let held = other.tx().unwrap();
        let started = Instant::now();
        inbox.post(event(1));
        inbox.post(failed("a"));
        inbox.post(event(2));
        std::thread::sleep(Duration::from_secs(1));
        assert!(inbox.listing().entries.is_empty(), "nothing written while the lock is held");
        drop(held);
        assert!(inbox.settle());
        assert!(started.elapsed() >= Duration::from_secs(1));
        let listing = inbox.listing();
        assert_eq!(notices(&listing), [event(2), failed("a"), event(1)]);
        drop(other);
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The close wakes a writer trying a busy catalog again: once the lock is let go, what is queued is written and the
    // close returns, long before the shutdown's deadline; and it is in the catalog.
    #[test]
    fn the_close_wakes_a_retrying_writer_and_drains() {
        let (inbox, path, dir) = inbox("inbox-close");
        assert!(inbox.settle());
        let mut other = open(&path);
        let held = other.tx().unwrap();
        inbox.post(failed("at the end"));
        std::thread::sleep(Duration::from_millis(300));
        let closing = inbox.clone();
        let started = Instant::now();
        let closer = std::thread::spawn(move || closing.close());
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
        closer.join().unwrap();
        assert!(started.elapsed() < SHUTDOWN_DRAIN, "{:?}", started.elapsed());
        let kept = other.inbox_entries(10, false).unwrap();
        assert_eq!(kept.iter().map(|e| e.notice.clone()).collect::<Vec<_>>(), [failed("at the end")]);
        drop(other);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A catalog that stays busy past the shutdown's deadline: the close returns after it, and what is left is not
    // written (it is logged).
    #[test]
    fn the_close_gives_up_after_its_deadline() {
        let (inbox, path, dir) = inbox("inbox-deadline");
        assert!(inbox.settle());
        let mut other = open(&path);
        let held = other.tx().unwrap();
        inbox.post(event(1));
        let started = Instant::now();
        inbox.close();
        let took = started.elapsed();
        assert!(took >= SHUTDOWN_DRAIN && took < SHUTDOWN_DRAIN + Duration::from_secs(3), "{took:?}");
        drop(held);
        assert!(other.inbox_entries(10, false).unwrap().is_empty());
        drop(other);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A notice is logged when it is posted, before anything is written (the catalog is locked meanwhile), and a state
    // posted again soon after is not logged again.
    #[test]
    fn a_notice_is_logged_before_it_is_written_and_throttled() {
        let (inbox, path, dir) = inbox("inbox-log");
        assert!(inbox.settle());
        let feed = pg_log::subscribe();
        let mut other = open(&path);
        let held = other.tx().unwrap();
        let unique = format!("logged once {}", std::process::id());
        inbox.post(failed(&unique));
        inbox.post(failed(&unique));
        let logged = feed.new.try_iter().filter(|r| r.message.contains(&unique)).count();
        assert_eq!(logged, 1);
        assert!(inbox.listing().entries.is_empty());
        drop(held);
        assert!(inbox.settle());
        drop(other);
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The notices posted before anyone subscribed reach the first subscriber, toasted; a later subscriber gets only
    // what comes after it.
    #[test]
    fn the_first_subscriber_gets_the_startup_notices() {
        let (inbox, _, dir) = inbox("inbox-first");
        inbox.post(event(1));
        assert!(inbox.settle());
        let first = inbox.subscribe();
        let later = inbox.subscribe();
        let items = first.take().items;
        assert!(items.iter().any(|e| matches!(e, InboxEvent::Toast(n) if *n == event(1))));
        assert!(later.take().items.is_empty());
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A subscriber that doesn't keep up loses the oldest events (counted), and the listing is there to read.
    #[test]
    fn a_lagged_subscriber_reads_the_listing() {
        let (inbox, _, dir) = inbox("inbox-lagged");
        let sub = inbox.subscribe();
        for i in 0..140 {
            inbox.post(event(i));
            assert!(inbox.settle());
        }
        let taken = sub.take();
        assert!(taken.lagged > 0);
        let listing = inbox.listing();
        assert_eq!(listing.entries.first().map(|e| e.notice.clone()), Some(event(139)));
        assert!(matches!(taken.items.last(), Some(InboxEvent::Changed(l)) if **l == *listing));
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A clear of what isn't stored takes no write lock: another connection sees no change.
    #[test]
    fn a_clear_of_nothing_writes_nothing() {
        let (inbox, path, dir) = inbox("inbox-clear-nothing");
        assert!(inbox.settle());
        let other = open(&path);
        let before = other.data_version().unwrap();
        inbox.clear(Key::all("listing-failed", None).to_buf());
        inbox.clear(Key::all("edit-failed", None).to_buf());
        assert!(inbox.settle());
        assert_eq!(other.data_version().unwrap(), before);
        drop(other);
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The ledgers' counts are read only while the inbox is viewed.
    #[test]
    fn a_closed_inbox_reads_no_counts() {
        let (inbox, path, dir) = inbox("inbox-viewing");
        let failure = a_file(&path, &dir);
        inbox.thumb_failed(failure);
        assert!(inbox.settle());
        let ledger = |inbox: &Inbox| {
            inbox
                .listing()
                .entries
                .iter()
                .find(|e| matches!(e.id, EntryId::Ledger { kind: LedgerKind::File(FilePass::Thumbs), .. }))
                .cloned()
                .expect("the thumbnails' entry")
        };
        assert_eq!(ledger(&inbox).counts, None);
        inbox.set_viewing(true);
        assert!(inbox.settle());
        assert_eq!(total(&ledger(&inbox)), Some(1));
        inbox.set_viewing(false);
        assert!(inbox.settle());
        assert_eq!(ledger(&inbox).counts, None);
        // A file made: it leaves the ledger.
        inbox.thumb_made(FileId::new(1));
        assert!(inbox.settle());
        assert!(inbox.listing().entries.is_empty());
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
