//! The inbox (docs/NOTIFICATIONS.md): what the app tells the user, kept in the catalog until they dismiss it. A service:
//! it holds the catalog's path, and its own connection on its own thread; it never reaches the `App`.
//!
//! Every producer posts a [`Notice`] ([`Inbox::post`]), from any thread: the notice is logged there and then (so a
//! notice the inbox can't keep is still in the log), and queued for the writer. The writer keeps it as its lifecycle says
//! (`pg_catalog::inbox`), and after every change it reads the inbox and publishes what it lists ([`InboxEvent::Changed`]),
//! with a [`InboxEvent::Toast`] for each entry that is news: new, or posted again saying something new. What another
//! process changes (the command line) is found by the catalog's `data_version`, looked at every [`POLL`] and when poked
//! ([`Inbox::poke`]). The window's marks and dismissals are queued the same way; it never reads the catalog for the
//! inbox.
//!
//! A transient notice (feedback on what the user just did) is only logged and toasted. A notice that the queue has no
//! room for, or that couldn't be written, is counted, and a [`Notice::NoticesLost`] says how many.
//!
//! A pass's failure ledger (`pg_catalog::ledger`) is an entry of its own, listed with the rows. The writer also keeps
//! the thumbnails' ledger for the thumbnails the window makes ([`Inbox::thumb_failed`], [`Inbox::thumb_made`]), and
//! carries out the user's Retry ([`Inbox::retry`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pg_catalog::Catalog;
use pg_catalog::inbox::{self, Entry, EntryId, Mark};
use pg_catalog::ledger::{self, Failure};
use pg_jobs::events::{Bus, Subscription};
use pg_library::runner::NoticeSink;
use pg_model::ids::FileId;
use pg_model::notice::{FilePass, KeyBuf, Level, Lifecycle, Notice};

use crate::error::{Result, failed};

/// How many commands wait for the writer before a post is counted as lost (a catalog locked for long by a scan).
const QUEUE: usize = 1024;

/// How often the writer looks for what another process changed.
pub const POLL: Duration = Duration::from_secs(30);

/// How many entries the inbox lists, newest first.
pub const LISTED: usize = 200;

/// A kept notice posted again within this long is not logged again (an edit failing at every key).
const LOG_AGAIN: Duration = Duration::from_secs(60);

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

/// What the writer is asked to do.
enum Command {
    Post(Notice, i64),
    Replace(KeyBuf, Vec<Notice>, i64),
    MarkRead(Vec<EntryId>),
    /// Every row, and the ledgers as the last listing had them.
    MarkAllRead(Vec<EntryId>),
    Dismiss(EntryId),
    DismissAll(Vec<EntryId>),
    /// The user's Retry of a pass's failed files, then what follows it (the job woken).
    Retry(FilePass, Box<dyn FnOnce() + Send>),
    /// A thumbnail the window made failed, or was made (its failure is gone).
    ThumbFailed(Failure),
    ThumbMade(FileId),
    /// Look for what another process changed.
    Poke,
}

pub struct Inbox {
    /// The writer's queue; `None` once the inbox is closed.
    sender: Mutex<Option<SyncSender<Command>>>,
    /// Posts that weren't kept since the writer last said so.
    lost: Arc<AtomicU64>,
    events: Arc<Bus<InboxEvent>>,
    /// The subscription taken when the inbox started, for the first subscriber: the notices of the app's opening reach it.
    first: Mutex<Option<Subscription<InboxEvent>>>,
    listing: Arc<Mutex<Arc<Listing>>>,
    /// When each kept notice was last logged, by its key.
    logged: Mutex<BTreeMap<String, Instant>>,
    /// The kinds of kept notices the inbox may hold: a clear of another kind is not sent (the window clears a kind at
    /// every read that works, which must cost nothing when there is nothing to clear).
    held: Arc<Mutex<BTreeSet<&'static str>>>,
    /// The thumbnails' ledger may hold files: a thumbnail the window makes is sent to clear its file only then.
    thumb_failures: Arc<AtomicBool>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Inbox {
    /// Start the inbox of the catalog at `catalog_path` (prepared): its writer opens its own connection. An error if the
    /// thread can't start.
    pub(crate) fn start(catalog_path: PathBuf) -> Result<Arc<Inbox>> {
        let (sender, receiver) = sync_channel(QUEUE);
        let events: Arc<Bus<InboxEvent>> = Arc::new(Bus::new(256));
        let first = Mutex::new(Some(events.subscribe()));
        let lost = Arc::new(AtomicU64::new(0));
        let listing = Arc::new(Mutex::new(Arc::new(Listing::default())));
        let held = Arc::new(Mutex::new(BTreeSet::new()));
        let thumb_failures = Arc::new(AtomicBool::new(true));
        let writer = Writer {
            events: events.clone(),
            listing: listing.clone(),
            lost: lost.clone(),
            held: held.clone(),
            thumb_failures: thumb_failures.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("inbox".into())
            .spawn(move || writer.run(&catalog_path, &receiver))
            .map_err(|e| failed(format!("can't start the inbox: {e}")))?;
        Ok(Arc::new(Inbox {
            sender: Mutex::new(Some(sender)),
            lost,
            events,
            first,
            listing,
            logged: Mutex::new(BTreeMap::new()),
            held,
            thumb_failures,
            writer: Mutex::new(Some(thread)),
        }))
    }

    /// Tell the user `notice`: it is logged now, and kept as its lifecycle says (a transient one is only toasted).
    pub fn post(&self, notice: Notice) {
        self.log(&notice);
        match notice.lifecycle() {
            Lifecycle::Transient => self.events.publish(InboxEvent::Toast(notice)),
            // Built from the failure ledger when the inbox is read, never posted.
            Lifecycle::Ledger(_) => {}
            Lifecycle::State(key) => {
                self.held().insert(key.code);
                self.send(Command::Post(notice, now()));
            }
            Lifecycle::Event => self.send(Command::Post(notice, now())),
        }
    }

    /// The kept notices of `scope` are now `notices` (`pg_catalog::inbox::replace`).
    pub fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
        if notices.is_empty() {
            self.clear(scope);
            return;
        }
        for notice in &notices {
            self.log(notice);
        }
        self.held().insert(scope.code);
        self.send(Command::Replace(scope, notices, now()));
    }

    /// The kept notices `key` names no longer hold. Nothing is sent when the inbox holds none of its kind.
    pub fn clear(&self, key: KeyBuf) {
        {
            let mut held = self.held();
            if !held.contains(key.code) {
                return;
            }
            // Every one of the kind goes: until one is posted again, there is nothing to clear.
            if key.root.is_none() && key.part.is_none() {
                held.remove(key.code);
            }
        }
        self.send(Command::Replace(key, Vec::new(), now()));
    }

    fn held(&self) -> std::sync::MutexGuard<'_, BTreeSet<&'static str>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mark the entries `ids` read.
    pub fn mark_read(&self, ids: Vec<EntryId>) {
        self.send(Command::MarkRead(ids));
    }

    /// Mark every entry read: the ledgers as the last listing had them (a failure after it stays new).
    pub fn mark_all_read(&self) {
        self.send(Command::MarkAllRead(self.listed_ledgers()));
    }

    pub fn dismiss(&self, id: EntryId) {
        self.send(Command::Dismiss(id));
    }

    /// Dismiss every entry, the ledgers as the last listing had them.
    pub fn dismiss_all(&self) {
        self.send(Command::DismissAll(self.listed_ledgers()));
    }

    fn listed_ledgers(&self) -> Vec<EntryId> {
        self.listing().entries.iter().map(|e| e.id).filter(|id| matches!(id, EntryId::Ledger { .. })).collect()
    }

    /// Try `pass`'s failed files again (`pg_catalog::ledger::retry`); `then` runs once that is written (the job is woken
    /// to make them).
    pub fn retry(&self, pass: FilePass, then: Box<dyn FnOnce() + Send>) {
        self.send(Command::Retry(pass, then));
    }

    /// A thumbnail the window asked for couldn't be made: the thumbnails' ledger records it.
    pub fn thumb_failed(&self, failure: Failure) {
        self.thumb_failures.store(true, Ordering::Relaxed);
        self.send(Command::ThumbFailed(failure));
    }

    /// A thumbnail the window asked for was made: the file leaves the thumbnails' ledger, if it is there. Nothing is sent
    /// while the ledger is empty.
    pub fn thumb_made(&self, file_id: FileId) {
        if self.thumb_failures.load(Ordering::Relaxed) {
            self.send(Command::ThumbMade(file_id));
        }
    }

    /// Look for what another process changed now, not at the next [`POLL`] (the window was activated, a pass ended).
    pub fn poke(&self) {
        self.send(Command::Poke);
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

    /// Write what is queued and stop the writer; later posts are only logged. Called at shutdown, before the catalog's
    /// connections close.
    pub(crate) fn close(&self) {
        drop(self.sender.lock().unwrap_or_else(PoisonError::into_inner).take());
        if let Some(writer) = self.writer.lock().unwrap_or_else(PoisonError::into_inner).take()
            && writer.join().is_err()
        {
            pg_log::error("inbox", "the inbox's writer panicked");
        }
    }

    fn send(&self, command: Command) {
        let sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = sender.as_ref() else { return };
        match sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(Command::Post(..) | Command::Replace(..) | Command::ThumbFailed(_))) => {
                if self.lost.fetch_add(1, Ordering::Relaxed) == 0 {
                    pg_log::warn("inbox", "the inbox's queue is full: notices are not kept");
                }
            }
            // A mark or a poke the writer has no room for: the next one does it.
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Log `notice` as the developer reads it (its stored form); a kept one posted again soon after is not logged again.
    fn log(&self, notice: &Notice) {
        if let Lifecycle::State(key) = notice.lifecycle() {
            let key = format!("{key:?}");
            let mut logged = self.logged.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            if logged.get(&key).is_some_and(|at| now.duration_since(*at) < LOG_AGAIN) {
                return;
            }
            logged.insert(key, now);
        }
        let text = serde_json::to_string(notice).unwrap_or_else(|e| format!("{notice:?} ({e})"));
        let target = notice.source().code();
        match notice.level() {
            Level::Info => pg_log::info(target, &text),
            Level::Warning => pg_log::warn(target, &text),
            Level::Error => pg_log::error(target, &text),
        }
    }
}

/// The library's job posts into the inbox.
impl NoticeSink for Inbox {
    fn post(&self, notice: Notice) {
        Inbox::post(self, notice);
    }

    fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
        Inbox::replace(self, scope, notices);
    }
}

/// The writer's thread: what it publishes to.
struct Writer {
    events: Arc<Bus<InboxEvent>>,
    listing: Arc<Mutex<Arc<Listing>>>,
    lost: Arc<AtomicU64>,
    held: Arc<Mutex<BTreeSet<&'static str>>>,
    thumb_failures: Arc<AtomicBool>,
}

impl Writer {
    fn run(&self, catalog_path: &std::path::Path, commands: &Receiver<Command>) {
        let catalog = match Catalog::open(catalog_path, pg_platform::native::PATH_RULES) {
            Ok(catalog) => catalog,
            Err(e) => {
                pg_log::error("inbox", &format!("the inbox can't open the catalog, so nothing is kept: {e:#}"));
                return;
            }
        };
        if let Err(e) = inbox::prune(&catalog, now()) {
            pg_log::warn("inbox", &format!("the inbox couldn't be pruned: {e:#}"));
        }
        // What the inbox held when the app started is not news: it is listed, not toasted.
        let mut seen = BTreeMap::new();
        self.refresh(&catalog, &mut seen, false);
        let mut version = catalog.data_version().ok();
        loop {
            let first = match commands.recv_timeout(POLL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let mut wrote = false;
            let mut failed = false;
            for command in first.into_iter().chain(commands.try_iter()) {
                match apply(&catalog, command) {
                    Ok(changed) => wrote |= changed,
                    Err(e) => {
                        failed = true;
                        self.lost.fetch_add(1, Ordering::Relaxed);
                        pg_log::warn("inbox", &format!("the inbox couldn't be written: {e:#}"));
                    }
                }
            }
            // How many weren't kept, said once a write has gone through again.
            if !failed {
                let lost = self.lost.swap(0, Ordering::Relaxed);
                if lost > 0 {
                    match inbox::post(&catalog, &Notice::NoticesLost { count: lost }, now()) {
                        Ok(_) => wrote = true,
                        Err(_) => {
                            self.lost.fetch_add(lost, Ordering::Relaxed);
                        }
                    }
                }
            }
            // Read outside a transaction: it moves only when another connection has committed.
            let now_version = catalog.data_version().ok();
            if wrote || now_version != version {
                version = now_version;
                self.refresh(&catalog, &mut seen, true);
            }
        }
    }

    /// Read the inbox and publish what it lists; with `toast`, each entry that is news is toasted. `seen` is what the last
    /// read found.
    fn refresh(&self, catalog: &Catalog, seen: &mut BTreeMap<EntryId, Mark>, toast: bool) {
        let read = inbox::marks(catalog).and_then(|marks| Ok((marks, inbox::entries(catalog, LISTED)?)));
        let (marks, entries) = match read {
            Ok(read) => read,
            Err(e) => {
                pg_log::warn("inbox", &format!("the inbox couldn't be read: {e:#}"));
                return;
            }
        };
        // A ledger is one entry whatever it was listed up to: news when it turns unread.
        let news: Vec<EntryId> = marks
            .iter()
            .filter(|m| !m.read && seen.get(&m.id.entry()).is_none_or(|before| before.read))
            .map(|m| m.id)
            .collect();
        let unread = u64::try_from(marks.iter().filter(|m| !m.read).count()).unwrap_or(u64::MAX);
        *seen = marks.into_iter().map(|m| (m.id.entry(), m)).collect();
        match ledger::any(catalog, FilePass::Thumbs) {
            Ok(any) => self.thumb_failures.store(any, Ordering::Relaxed),
            Err(e) => pg_log::warn("inbox", &format!("the thumbnails' ledger couldn't be read: {e:#}")),
        }
        // The kinds it holds (dismissed ones aside, which a clear may leave): another process's, and the earlier runs'.
        self.held.lock().unwrap_or_else(PoisonError::into_inner).extend(
            entries.iter().filter(|e| matches!(e.notice.lifecycle(), Lifecycle::State(_))).map(|e| e.notice.code()),
        );
        let listing = Arc::new(Listing { entries, unread });
        *self.listing.lock().unwrap_or_else(PoisonError::into_inner) = listing.clone();
        if toast {
            for entry in listing.entries.iter().filter(|e| news.contains(&e.id)) {
                // A ledger's entry is never posted: it is logged when it is news.
                if let EntryId::Ledger { .. } = entry.id {
                    let text = serde_json::to_string(&entry.notice).unwrap_or_else(|e| format!("{e}"));
                    pg_log::warn(entry.notice.source().code(), &format!("{text} ({} files)", entry.count));
                }
                self.events.publish(InboxEvent::Toast(entry.notice.clone()));
            }
        }
        self.events.publish(InboxEvent::Changed(listing));
    }
}

/// Carry out `command`; whether it may have changed the inbox.
fn apply(catalog: &Catalog, command: Command) -> pg_catalog::Result<bool> {
    let now = now();
    match command {
        Command::Post(notice, at) => inbox::post(catalog, &notice, at),
        Command::Replace(scope, notices, at) => inbox::replace(catalog, &scope.key(), &notices, at).map(|()| true),
        Command::MarkRead(ids) => inbox::mark_read(catalog, &ids, now).map(|()| true),
        Command::MarkAllRead(ledgers) => inbox::mark_all_read(catalog, &ledgers, now).map(|()| true),
        Command::Dismiss(id) => inbox::dismiss(catalog, id, now).map(|()| true),
        Command::DismissAll(ledgers) => inbox::dismiss_all(catalog, &ledgers, now).map(|()| true),
        Command::Retry(pass, then) => {
            ledger::retry(catalog, pass)?;
            then();
            Ok(true)
        }
        Command::ThumbFailed(failure) => ledger::record(catalog, FilePass::Thumbs, &[failure], now).map(|()| true),
        Command::ThumbMade(file) => ledger::clear(catalog, FilePass::Thumbs, &[file]).map(|()| true),
        Command::Poke => Ok(false),
    }
}

/// Now, in Unix seconds (the catalog's times).
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::{EditOp, Key};

    fn inbox(name: &str) -> (Arc<Inbox>, PathBuf) {
        let dir = pg_testkit::test_folder("scratch_app", name).unwrap();
        let catalog = pg_library::open_catalog(&dir).unwrap();
        drop(catalog);
        (Inbox::start(pg_library::catalog_path(&dir)).unwrap(), dir)
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

    // A post is kept and toasted once; the same again is counted and not toasted; a dismissal and a clear take it out of
    // the listing. The first subscriber has every listing from the start, the baseline untoasted.
    #[test]
    fn a_post_is_listed_and_toasted_once() {
        let (inbox, dir) = inbox("inbox-post");
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
        assert_eq!(listing.entries[0].count, 2);
        inbox.mark_read(vec![listing.entries[0].id]);
        assert_eq!(next(&sub).1.unread, 0);
        inbox.clear(Key::all("edit-failed", None).to_buf());
        assert!(next(&sub).1.entries.is_empty());
        // A transient notice is only toasted.
        inbox.post(Notice::FileNoLongerInLibrary);
        assert!(matches!(sub.recv(), Some(InboxEvent::Toast(Notice::FileNoLongerInLibrary))));
        inbox.close();
        // After the close a post is only logged.
        inbox.post(failed("b"));
        assert_eq!(inbox.listing().entries.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // What another connection writes is found by a poke, and toasted.
    #[test]
    fn another_connections_post_is_found_by_a_poke() {
        let (inbox, dir) = inbox("inbox-poke");
        let sub = inbox.subscribe();
        next(&sub);
        let other = Catalog::open(&pg_library::catalog_path(&dir), pg_platform::native::PATH_RULES).unwrap();
        inbox::post(&other, &failed("elsewhere"), now()).unwrap();
        inbox.poke();
        let (toasts, listing) = next(&sub);
        assert_eq!(toasts, [failed("elsewhere")]);
        assert_eq!(listing.unread, 1);
        drop(other);
        inbox.close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
