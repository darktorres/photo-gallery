//! The app's log (docs/NOTIFICATIONS.md): what went wrong that the user needn't act on (diagnostics), and every notice
//! the inbox shows, recorded by its code. A record is a JSON line in `gallery.log` in the data folder (`init`), which is
//! capped and rotated (`file`), and the newest records are also kept in memory from the process's start (`subscribe`),
//! so the window's log view shows this run's records at once and follows the new ones; `read` adds the older ones from
//! the files. Before `init`, and when the file can't be written, a record goes to stderr too: none is dropped.
//!
//! Only the window writes `gallery.log`; the command line prints to stderr.

mod file;
mod record;

use std::cell::Cell;
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

use file::LogFile;
pub use file::{FILE_CAP, FILES};
use pg_model::notice::Level;
pub use record::{DETAIL_CAP, MESSAGE_CAP, Record, parse};

/// The most records, and bytes of their texts, kept in memory.
pub const RING_RECORDS: usize = 5000;
pub const RING_BYTES: usize = 4 * 1024 * 1024;

/// The process's log.
static LOG: LazyLock<Logger> = LazyLock::new(Logger::new);

/// Write the log to `path` from now on, and record the session's start with `about` (the version, the data folder).
/// Called once, at the start; a second call is ignored.
pub fn init(path: PathBuf, about: &str) {
    LOG.init(path, about);
}

pub fn info(target: &str, message: &str) {
    LOG.log(Level::Info, target, message, None);
}

pub fn warn(target: &str, message: &str) {
    LOG.log(Level::Warning, target, message, None);
}

pub fn error(target: &str, message: &str) {
    LOG.log(Level::Error, target, message, None);
}

/// A record with a detail (a notice's details, a list of files).
pub fn record(level: Level, target: &str, message: &str, detail: Option<&str>) {
    LOG.log(level, target, message, detail);
}

/// A panic's record, from the panic hook: written even when the panic came while the log was being written (it then
/// skips the lock its own thread holds). The error says why the file couldn't take it.
pub fn panic(message: &str, backtrace: &str) -> std::io::Result<()> {
    LOG.panic(message, backtrace)
}

/// The log's file, once `init` set it.
pub fn path() -> Option<PathBuf> {
    LOG.file.get().map(|f| f.path().to_path_buf())
}

/// This run's records kept in memory, oldest first, and from then on each new one (see [`Feed`]).
pub fn subscribe() -> Feed {
    LOG.subscribe()
}

/// The records of the log's files at `path`, oldest first, at most `max` of the newest.
pub fn read(path: &Path, max: usize) -> Vec<Record> {
    let mut records = parse(&file::read_all(path));
    let extra = records.len().saturating_sub(max);
    records.drain(..extra);
    records
}

/// What a subscriber gets: the records kept in memory when it subscribed, then each new one, in order, with none
/// missed and none twice.
pub struct Feed {
    /// This run's records kept in memory, oldest first.
    pub kept: Vec<Record>,
    /// Each record logged after.
    pub new: Receiver<Record>,
    /// This run's identity and the first `seq` that `kept` and `new` hold: a record of the files with them, or after,
    /// is one of theirs ([`Feed::merge`]).
    pid: u32,
    session: u64,
    from_seq: u64,
}

impl Feed {
    /// The records of the files (`read`) with this run's kept ones: the files' records, less the ones of this run the
    /// feed already has, then the kept ones. A record of this run written to the file only (a panic while its thread
    /// held the ring, [`Record::file_only`]) is in neither `kept` nor `new`, so it is taken from the files whatever its
    /// number.
    pub fn merge(&self, from_files: Vec<Record>) -> Vec<Record> {
        let mut all: Vec<Record> = from_files
            .into_iter()
            .filter(|r| r.file_only || !(r.pid == self.pid && r.session == self.session && r.seq >= self.from_seq))
            .collect();
        all.extend(self.kept.iter().cloned());
        all
    }
}

/// A log: its file once set, the records in memory, and the subscribers.
struct Logger {
    file: OnceLock<LogFile>,
    /// Serialises the writes to the file.
    writing: Mutex<()>,
    /// The records in memory and the subscribers: taken only through [`RingGuard`].
    ring: Mutex<Ring>,
    seq: AtomicU64,
    pid: u32,
    session: u64,
    /// A test's fault, run (once) inside `keep_in` with the ring held: a panic there.
    #[cfg(test)]
    fault: Mutex<Option<fn(&Logger)>>,
}

thread_local! {
    /// The ring this thread holds, by its address (0 for none): set by [`RingGuard::lock`], cleared when the guard is
    /// dropped, unwinding included. The panic hook reads it, so a panic raised while its thread holds the ring doesn't
    /// take it again (a std `Mutex` locked twice by one thread may deadlock).
    static HOLDING: Cell<usize> = const { Cell::new(0) };
}

/// The ring, held: every acquisition of it goes through this (`keep`, the panic's `keep_unless_held`, `subscribe`), so
/// the thread's flag is never set by one path and missed by another.
struct RingGuard<'a> {
    ring: MutexGuard<'a, Ring>,
}

impl<'a> RingGuard<'a> {
    /// The ring, waiting for another thread that holds it. A panic while it was held left it as it was (a record is
    /// whole or not there), so a poisoned lock is taken all the same.
    fn lock(mutex: &'a Mutex<Ring>) -> RingGuard<'a> {
        let ring = lock(mutex);
        HOLDING.set(address(mutex));
        RingGuard { ring }
    }

    /// Whether this thread holds `mutex`'s ring.
    fn held_here(mutex: &Mutex<Ring>) -> bool {
        HOLDING.get() == address(mutex)
    }
}

impl Drop for RingGuard<'_> {
    fn drop(&mut self) {
        // Before the lock is let go (the fields drop after this): no other thread reads this thread's flag.
        HOLDING.set(0);
    }
}

impl Deref for RingGuard<'_> {
    type Target = Ring;

    fn deref(&self) -> &Ring {
        &self.ring
    }
}

impl DerefMut for RingGuard<'_> {
    fn deref_mut(&mut self) -> &mut Ring {
        &mut self.ring
    }
}

/// A ring's identity for the thread's flag: its address, never 0.
fn address(mutex: &Mutex<Ring>) -> usize {
    std::ptr::from_ref(mutex).addr()
}

#[derive(Default)]
struct Ring {
    records: VecDeque<Record>,
    bytes: usize,
    subscribers: Vec<Sender<Record>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while it was held leaves the ring or the file as they were: a record is whole or not there.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
}

/// `2026-10-30 17:04:16`, local time.
fn local_time() -> String {
    let t = pg_platform::clock::local_time();
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

impl Logger {
    fn new() -> Logger {
        Logger {
            file: OnceLock::new(),
            writing: Mutex::new(()),
            ring: Mutex::new(Ring::default()),
            seq: AtomicU64::new(0),
            pid: std::process::id(),
            session: now_ms(),
            #[cfg(test)]
            fault: Mutex::new(None),
        }
    }

    fn init(&self, path: PathBuf, about: &str) {
        self.init_capped(path, FILE_CAP, about);
    }

    fn init_capped(&self, path: PathBuf, cap: u64, about: &str) {
        if self.file.set(LogFile::new(path, cap)).is_ok() {
            self.log(Level::Info, "session", "start", Some(about));
        }
    }

    fn make(&self, level: Level, target: &str, message: &str, detail: Option<&str>) -> Record {
        Record {
            t_ms: Some(now_ms()),
            local: local_time(),
            level,
            target: target.to_string(),
            message: record::capped(message, MESSAGE_CAP),
            detail: detail.map(|d| record::capped(d, DETAIL_CAP)),
            pid: self.pid,
            session: self.session,
            seq: 0,
            file_only: false,
        }
    }

    fn log(&self, level: Level, target: &str, message: &str, detail: Option<&str>) {
        let record = self.keep(self.make(level, target, message, detail));
        let written = {
            let _writing = lock(&self.writing);
            self.file.get().map(|file| file.append(&record.line()))
        };
        if !matches!(written, Some(Ok(()))) {
            fallback(&record);
        }
    }

    fn panic(&self, message: &str, backtrace: &str) -> std::io::Result<()> {
        let record = self.keep_unless_held(self.make(Level::Error, "panic", message, Some(backtrace)));
        let Some(file) = self.file.get() else {
            fallback(&record);
            return Err(std::io::Error::other("the log has no file yet"));
        };
        // The thread that panicked may hold the lock (a panic inside a write): then it writes without it.
        let _writing = match self.writing.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        };
        file.append(&record.line())
    }

    /// Number `record`, keep it in memory and send it to the subscribers, all under the ring's lock, so the numbers,
    /// the ring and each subscriber's order agree.
    fn keep(&self, record: Record) -> Record {
        let mut ring = RingGuard::lock(&self.ring);
        self.keep_in(&mut ring, record)
    }

    /// As `keep`, for the panic hook. Another thread that holds the ring is waited for, as `keep` waits. Only when this
    /// thread holds it (the panic was raised inside `keep` or `subscribe`), which taking it again could deadlock, is
    /// the record numbered and marked [`Record::file_only`], for the file alone: a subscriber's merge finds it there.
    fn keep_unless_held(&self, record: Record) -> Record {
        if RingGuard::held_here(&self.ring) {
            return Record { seq: self.seq.fetch_add(1, Ordering::Relaxed), file_only: true, ..record };
        }
        self.keep(record)
    }

    fn keep_in(&self, ring: &mut Ring, mut record: Record) -> Record {
        #[cfg(test)]
        if let Some(fault) = lock(&self.fault).take() {
            fault(self);
        }
        record.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        ring.bytes += record.bytes();
        ring.records.push_back(record.clone());
        while ring.records.len() > RING_RECORDS || ring.bytes > RING_BYTES {
            let Some(old) = ring.records.pop_front() else { break };
            ring.bytes = ring.bytes.saturating_sub(old.bytes());
        }
        ring.subscribers.retain(|s| s.send(record.clone()).is_ok());
        record
    }

    fn subscribe(&self) -> Feed {
        let mut ring = RingGuard::lock(&self.ring);
        let (send, new) = channel();
        ring.subscribers.push(send);
        let kept: Vec<Record> = ring.records.iter().cloned().collect();
        let from_seq = kept.first().map_or_else(|| self.seq.load(Ordering::Relaxed), |r| r.seq);
        Feed { kept, new, pid: self.pid, session: self.session, from_seq }
    }
}

/// A record the file didn't take, on stderr: a terminal still shows it.
#[expect(clippy::print_stderr, reason = "the fallback: a record the log can't take still reaches a terminal")]
fn fallback(record: &Record) {
    eprint!("{}", record.plain());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger(name: &str, cap: u64) -> (Logger, PathBuf) {
        let dir = pg_testkit::run_folder("scratch_log", name).unwrap();
        let path = dir.join("gallery.log");
        let log = Logger::new();
        log.init_capped(path.clone(), cap, "test");
        (log, path)
    }

    #[test]
    fn a_session_starts_with_its_record_and_each_record_follows_in_order() {
        let (log, path) = logger("order", FILE_CAP);
        log.log(Level::Warning, "library", "one", None);
        log.log(Level::Error, "video", "two", Some("why"));
        let records = read(&path, 100);
        let messages: Vec<&str> = records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages, ["start", "one", "two"]);
        assert_eq!(records[0].detail.as_deref(), Some("test"));
        assert_eq!(records.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(records[2].level, Level::Error);
    }

    /// The ring holds this run's newest records, bounded by their count and by their bytes.
    #[test]
    fn the_ring_is_bounded_by_count_and_bytes() {
        let log = Logger::new();
        for i in 0..RING_RECORDS + 10 {
            log.log(Level::Info, "t", &format!("{i}"), None);
        }
        let feed = log.subscribe();
        assert_eq!(feed.kept.len(), RING_RECORDS);
        assert_eq!(feed.kept[0].seq, 10);

        let big = Logger::new();
        let text = "x".repeat(MESSAGE_CAP);
        for _ in 0..(RING_BYTES / MESSAGE_CAP) + 5 {
            big.log(Level::Info, "t", &text, None);
        }
        let kept = big.subscribe().kept;
        assert!(kept.iter().map(Record::bytes).sum::<usize>() <= RING_BYTES);
        assert!(kept.len() < (RING_BYTES / MESSAGE_CAP) + 5);
    }

    /// A subscriber that reads the files after it subscribed gets each record once: the files' records of this run
    /// that the feed holds are left out, and the ones it has dropped (older) are kept.
    #[test]
    fn the_feed_and_the_files_merge_without_a_gap_or_a_double() {
        let (log, path) = logger("merge", FILE_CAP);
        for i in 0..5 {
            log.log(Level::Info, "t", &format!("before {i}"), None);
        }
        let feed = log.subscribe();
        log.log(Level::Info, "t", "after", None);
        let merged = feed.merge(read(&path, 100));
        let seqs: Vec<u64> = merged.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, [0, 1, 2, 3, 4, 5]);
        let next: Vec<String> = feed.new.try_iter().map(|r| r.message).collect();
        assert_eq!(next, ["after"]);
    }

    #[test]
    fn records_from_many_threads_are_each_one_line() {
        let (log, path) = logger("threads", FILE_CAP);
        let log = std::sync::Arc::new(log);
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let log = std::sync::Arc::clone(&log);
                std::thread::spawn(move || {
                    for i in 0..200 {
                        log.log(Level::Info, "t", &format!("{t}:{i} {}", "y".repeat(300)), None);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let records = read(&path, usize::MAX);
        assert_eq!(records.len(), 1 + 8 * 200);
        assert!(records.iter().all(|r| r.t_ms.is_some()));
        let seqs: std::collections::BTreeSet<u64> = records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs.len(), records.len());
    }

    #[test]
    fn a_rotated_log_reads_back_oldest_first() {
        let (log, path) = logger("rotated", 2000);
        for i in 0..40 {
            log.log(Level::Info, "t", &format!("{i:03}"), None);
        }
        let records = read(&path, usize::MAX);
        let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted);
        assert_eq!(seqs.last(), Some(&40));
        assert!(path.metadata().unwrap().len() <= 2000);
    }

    #[test]
    fn a_panic_is_written_while_the_lock_is_held() {
        let (log, path) = logger("panic", FILE_CAP);
        let held = lock(&log.writing);
        log.panic("panicked at x", "   0: main").unwrap();
        drop(held);
        let records = read(&path, 100);
        let last = records.last().unwrap();
        assert_eq!((last.target.as_str(), last.message.as_str()), ("panic", "panicked at x"));
        assert_eq!(last.detail.as_deref(), Some("   0: main"));
    }

    #[test]
    fn read_keeps_the_newest() {
        let (log, path) = logger("newest", FILE_CAP);
        for i in 0..10 {
            log.log(Level::Info, "t", &format!("{i}"), None);
        }
        let records = read(&path, 3);
        assert_eq!(records.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(), ["7", "8", "9"]);
    }

    /// A record logged while another thread holds the ring waits for it, and then is kept, sent and merged like any
    /// other, once each.
    #[test]
    fn a_record_logged_while_another_thread_holds_the_ring_arrives_once() {
        let (log, path) = logger("ring_held", FILE_CAP);
        let feed = log.subscribe();
        let held = RingGuard::lock(&log.ring);
        std::thread::scope(|s| {
            let waiting = s.spawn(|| log.log(Level::Warning, "t", "while held", None));
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert!(!waiting.is_finished(), "it waits for the ring");
            drop(held);
            waiting.join().unwrap();
        });
        let sent: Vec<String> = feed.new.try_iter().map(|r| r.message).collect();
        assert_eq!(sent, ["while held"]);
        let later = log.subscribe();
        assert_eq!(later.kept.iter().filter(|r| r.message == "while held").count(), 1);
        let merged = later.merge(read(&path, 100));
        assert_eq!(merged.iter().filter(|r| r.message == "while held").count(), 1);
    }

    /// What the panic hook does for a panic raised inside `keep`: it runs on the panicking thread, with the ring still
    /// held, before the unwinding.
    fn panic_in_keep(log: &Logger) {
        log.panic("panicked in keep", "   0: keep_in").unwrap();
        panic!("panicked in keep");
    }

    /// A panic raised while its thread holds the ring is written to the file only (taking the ring again could
    /// deadlock), and a subscriber's merge shows it once; the unwinding lets the ring go, and the log goes on.
    #[test]
    fn a_panic_inside_keep_is_written_to_the_file_and_merged_once() {
        let (log, path) = logger("keep_panic", FILE_CAP);
        let feed = log.subscribe();
        let fault: fn(&Logger) = panic_in_keep;
        *lock(&log.fault) = Some(fault);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            log.log(Level::Info, "t", "being kept", None);
        }));
        assert!(caught.is_err());
        assert!(!RingGuard::held_here(&log.ring), "the unwinding let the ring go");
        log.log(Level::Info, "t", "after", None);

        let records = read(&path, 100);
        let panics: Vec<&Record> = records.iter().filter(|r| r.target == "panic").collect();
        assert_eq!(panics.len(), 1);
        assert!(panics[0].file_only);
        assert_eq!(panics[0].message, "panicked in keep");
        let sent: Vec<Record> = feed.new.try_iter().collect();
        assert!(sent.iter().all(|r| r.target != "panic"), "a file-only record is not sent");
        assert_eq!(sent.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(), ["after"]);
        let merged = feed.merge(records);
        assert_eq!(merged.iter().filter(|r| r.target == "panic").count(), 1);
        assert!(merged.iter().all(|r| r.message != "after"), "a kept record is not taken from the files");
    }

    /// Panics written without the file's lock (another thread holds it): each is one append of one line, so every line
    /// reads back whole.
    #[test]
    fn panic_writes_without_the_lock_are_each_one_line() {
        let (log, path) = logger("panic_threads", FILE_CAP);
        let held = lock(&log.writing);
        std::thread::scope(|s| {
            for t in 0..2 {
                let log = &log;
                s.spawn(move || {
                    for i in 0..200 {
                        log.panic(&format!("{t}:{i}"), &"f".repeat(500)).unwrap();
                    }
                });
            }
        });
        drop(held);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1 + 2 * 200);
        for line in lines {
            assert!(serde_json::from_str::<Record>(line).is_ok(), "{line}");
        }
    }
}
