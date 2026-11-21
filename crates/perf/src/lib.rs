//! The perf log: the time each step of the background job and the import takes, with what it processed, written as JSON
//! lines to the file `PG_PERF_LOG` names (or echoed to stderr with `-`). Off until a sink is opened: then a span costs
//! one flag test and reads no clock.
//!
//! - [`span`]: a timed step, nested under the span open on its thread (`parent`). A span is `!Send`: it closes on the
//!   thread that opened it.
//! - [`Tally`]: a step repeated many times, possibly on several threads. Its totals are thread time, not wall time, and
//!   [`Span::attach`] puts them on the span that owns it.
//! - [`record`]: a duration the code has already measured, so the log and the code share one clock.
//! - [`summarize`]: the table `gallery perf` prints, read back from a file.
//!
//! The file is rotated like the app's log (`pg_log::LogFile`); one line is one `write`, so two processes appending to the
//! same file don't split each other's lines. A line that can't be written is counted ([`dropped`]), not retried.
//!
//! The library writes to no console: the echo is the command line's or the app's, through the sink it opens.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// The most bytes of a perf file before it is rotated (its older files are kept beside it, as the app log's are).
pub const FILE_CAP: u64 = 16 * 1024 * 1024;

/// The `program` every perf log's meta names. The command line and the window are one executable, so both write this
/// name (the CLI and the app's trace used to differ).
pub const PROGRAM: &str = "gallery";

/// Where the records go. A file, or an echo: each top-level span (one with no parent) is handed to the echo as its line.
enum Sink {
    File(pg_log::LogFile),
    Echo(fn(&str)),
}

/// The sink of this process, set once by [`open`] or [`open_echo`].
static SINK: OnceLock<Sink> = OnceLock::new();
/// Whether a span records anything: a sink is open, or a capture is running.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The records a capture took, in the order they closed.
static CAPTURED: Mutex<Vec<Value>> = Mutex::new(Vec::new());
/// Serialises captures: a capture's records are its own only while it runs alone.
static CAPTURE_LOCK: Mutex<()> = Mutex::new(());
/// Lines that could not be written since the process started.
static DROPPED: AtomicU64 = AtomicU64::new(0);
/// The ids of spans, unique in the process.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// The session of this process: the epoch milliseconds it opened the log at, and its pid.
static SESSION: OnceLock<(u64, u32)> = OnceLock::new();

thread_local! {
    /// The spans open on this thread, innermost last, each with the time its closed children took.
    static OPEN: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// Whether this thread's records go to the running [`capture`] (set on the thread that called it, only): another
    /// thread's records go to the sink, so a capture holds its own thread's records and no other test's.
    static CAPTURE_HERE: Cell<bool> = const { Cell::new(false) };
}

struct Frame {
    id: u64,
    child_ns: u64,
}

/// Whether a span records anything now: a sink is open, or a capture is running. Callers that would compute values only
/// for the log check this first.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// This process's session: the id its records carry (`session`), for [`summarize_session`].
pub fn session_id() -> u64 {
    SESSION.get().map_or(0, |s| s.0)
}

/// Opens the file at `path` as this process's sink, appending to it, and writes a `session` record with `meta` (what the
/// run is: the build, the data folder, the options, the catalog's counts). Creates the file's folder. Once per process:
/// a second open is an error, and the first sink stays.
pub fn open(path: &Path, meta: Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = pg_log::LogFile::new(path.to_path_buf(), FILE_CAP);
    install(Sink::File(file), meta)
}

/// Echoes this process's top-level spans through `echo` (one line each, the JSON of the record), with `meta` as the
/// session's. Once per process, like [`open`].
pub fn open_echo(echo: fn(&str), meta: Value) -> io::Result<()> {
    install(Sink::Echo(echo), meta)
}

fn install(sink: Sink, meta: Value) -> io::Result<()> {
    if SINK.set(sink).is_err() {
        return Err(io::Error::other("the perf log is already open in this process"));
    }
    let epoch =
        u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()).unwrap_or(u64::MAX);
    let pid = std::process::id();
    let _ = SESSION.set((epoch, pid));
    ENABLED.store(true, Ordering::Relaxed);
    let line = json!({
        "session": epoch,
        "pid": pid,
        "t_ms": pg_platform::clock::process_age_ms(),
        "id": 0,
        "name": "session",
        "parent": Value::Null,
        "dur_ms": 0.0,
        "self_ms": 0.0,
        "items": 0,
        "bytes": 0,
        "failed": 0,
        "fields": meta,
    });
    // Not a step: the echo leaves the session record out (a file keeps it, as the first line of the run).
    emit(line, false);
    Ok(())
}

/// Lines that could not be written since the process started. The command line says so when a run ends.
pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// Gives a record to the sink, or to the capture. Never panics; a failed write is counted.
fn emit(record: Value, top: bool) {
    if CAPTURE_HERE.with(Cell::get) {
        lock(&CAPTURED).push(record);
        return;
    }
    match SINK.get() {
        Some(Sink::File(file)) => {
            let mut line = record.to_string();
            line.push('\n');
            if file.append(&line).is_err() {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
        Some(Sink::Echo(echo)) if top => echo(&record.to_string()),
        _ => {}
    }
}

/// A mutex's guard, past a panic that poisoned it: the data is a log's, and a log keeps what it has.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Milliseconds of a duration, as the log holds times.
fn ms(d: Duration) -> f64 {
    pg_model::num::size(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)) / 1e6
}

fn ms_of_ns(ns: u64) -> f64 {
    pg_model::num::size(ns) / 1e6
}

/// A timed step: opened with [`span`], closed (and recorded) when dropped. Its fields are set while it is open: the
/// items, bytes and failures it processed, and any field of its own.
pub struct Span {
    live: Option<Live>,
    /// A span closes on the thread that opened it, which holds its place on the thread's stack.
    _not_send: PhantomData<*const ()>,
}

struct Live {
    id: u64,
    name: &'static str,
    parent: Option<u64>,
    start: Instant,
    t_ms: f64,
    items: u64,
    bytes: u64,
    failed: u64,
    paused_ms: Option<f64>,
    fields: Map<String, Value>,
}

/// Opens a span named `name` (a dotted name: `scan.walk`, `phase.import`), nested under the span open on this thread.
/// Inert when no sink is open: it reads no clock and allocates nothing.
pub fn span(name: &'static str) -> Span {
    if !enabled() {
        return Span { live: None, _not_send: PhantomData };
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let parent = OPEN.with(|open| {
        let mut open = open.borrow_mut();
        let parent = open.last().map(|f| f.id);
        open.push(Frame { id, child_ns: 0 });
        parent
    });
    let t_ms = pg_platform::clock::process_age_ms();
    Span {
        live: Some(Live {
            id,
            name,
            parent,
            start: Instant::now(),
            t_ms,
            items: 0,
            bytes: 0,
            failed: 0,
            paused_ms: None,
            fields: Map::new(),
        }),
        _not_send: PhantomData,
    }
}

impl Span {
    /// Items processed by this step.
    pub fn items(&mut self, n: u64) {
        if let Some(live) = &mut self.live {
            live.items = n;
        }
    }

    /// Bytes processed by this step.
    pub fn bytes(&mut self, n: u64) {
        if let Some(live) = &mut self.live {
            live.bytes = n;
        }
    }

    /// Items that failed in this step.
    pub fn failed(&mut self, n: u64) {
        if let Some(live) = &mut self.live {
            live.failed = n;
        }
    }

    /// The time this step was paused by the user, in milliseconds (its wall time includes it).
    pub fn paused(&mut self, paused: Duration) {
        if let Some(live) = &mut self.live {
            live.paused_ms = Some(ms(paused));
        }
    }

    /// A field of this step's own, under `key`.
    pub fn field(&mut self, key: &str, value: impl Into<Value>) {
        if let Some(live) = &mut self.live {
            live.fields.insert(key.to_string(), value.into());
        }
    }

    /// Puts a tally's totals on this step, under the tally's name. Its times are thread time: they can exceed the step's
    /// wall time when several threads worked in it.
    pub fn attach(&mut self, tally: &Tally) {
        if let Some(live) = &mut self.live {
            live.fields.insert(tally.name.to_string(), tally.snapshot());
        }
    }

    /// Puts the part of a tally's totals added since `since` on this step (a process-wide tally's share of this step).
    /// The largest single time is the tally's, not the part's: a maximum does not subtract.
    pub fn attach_since(&mut self, tally: &Tally, since: Mark) {
        if let Some(live) = &mut self.live {
            live.fields.insert(tally.name.to_string(), tally.since(since));
        }
    }
}

/// A tally's totals at one moment, for [`Span::attach_since`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Mark {
    count: u64,
    total_ns: u64,
    items: u64,
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(live) = self.live.take() else { return };
        let dur_ns = u64::try_from(live.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        // Pop this span's frame (it is the innermost, unless spans were closed out of order), and charge its duration to
        // the enclosing span's children.
        let child_ns = OPEN.with(|open| {
            let mut open = open.borrow_mut();
            let at = open.iter().rposition(|f| f.id == live.id);
            let child_ns = at.map_or(0, |i| open.remove(i).child_ns);
            if let Some(parent) = open.last_mut() {
                parent.child_ns = parent.child_ns.saturating_add(dur_ns);
            }
            child_ns
        });
        close(&live, dur_ns, child_ns);
    }
}

/// The record of a closed span, to the sink.
fn close(live: &Live, dur_ns: u64, child_ns: u64) {
    let mut record = json!({
        "session": SESSION.get().map_or(0, |s| s.0),
        "pid": SESSION.get().map_or(0, |s| s.1),
        "t_ms": live.t_ms,
        "id": live.id,
        "name": live.name,
        "parent": live.parent,
        "dur_ms": ms_of_ns(dur_ns),
        "self_ms": ms_of_ns(dur_ns.saturating_sub(child_ns)),
        "items": live.items,
        "bytes": live.bytes,
        "failed": live.failed,
        "fields": Value::Object(live.fields.clone()),
    });
    if let (Some(paused), Value::Object(map)) = (live.paused_ms, &mut record) {
        map.insert("paused_ms".to_string(), json!(paused));
    }
    emit(record, live.parent.is_none());
}

/// A duration the code has already measured, recorded under `name` inside the span open on this thread. It counts as a
/// child of that span, so the span's own time leaves it out.
pub fn record(name: &'static str, dur: Duration, fields: &[(&str, Value)]) {
    if !enabled() {
        return;
    }
    let dur_ns = u64::try_from(dur.as_nanos()).unwrap_or(u64::MAX);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let parent = OPEN.with(|open| {
        open.borrow_mut().last_mut().map(|f| {
            f.child_ns = f.child_ns.saturating_add(dur_ns);
            f.id
        })
    });
    let map: Map<String, Value> = fields.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect();
    let live = Live {
        id,
        name,
        parent,
        start: Instant::now(),
        // Every record's `t_ms` is when it opened: a measured duration opened `dur` before it was recorded.
        t_ms: pg_platform::clock::process_age_ms() - ms(dur),
        items: 0,
        bytes: 0,
        failed: 0,
        paused_ms: None,
        fields: map,
    };
    close(&live, dur_ns, 0);
}

/// A step repeated many times (a batch's write, a tier's query, a worker's wait), totalled across the threads that run
/// it. Its times are thread time: the sum over threads, which can exceed the wall time of the step it is attached to.
pub struct Tally {
    name: &'static str,
    count: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
    items: AtomicU64,
}

impl Tally {
    pub const fn new(name: &'static str) -> Tally {
        Tally {
            name,
            count: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
            items: AtomicU64::new(0),
        }
    }

    /// Times one occurrence of the step: the time from now until the timer is dropped. Inert when no sink is open.
    pub fn timer(&self) -> Timer<'_> {
        Timer { tally: self, start: enabled().then(Instant::now), items: 0 }
    }

    /// Adds one occurrence of `dur` that processed `items`.
    pub fn add(&self, dur: Duration, items: u64) {
        if !enabled() {
            return;
        }
        let ns = u64::try_from(dur.as_nanos()).unwrap_or(u64::MAX);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        self.items.fetch_add(items, Ordering::Relaxed);
    }

    /// The totals now, for [`Span::attach_since`]. Inert (zeros) when no sink is open.
    pub fn mark(&self) -> Mark {
        Mark {
            count: self.count.load(Ordering::Relaxed),
            total_ns: self.total_ns.load(Ordering::Relaxed),
            items: self.items.load(Ordering::Relaxed),
        }
    }

    fn snapshot(&self) -> Value {
        json!({
            "count": self.count.load(Ordering::Relaxed),
            "total_ms": ms_of_ns(self.total_ns.load(Ordering::Relaxed)),
            "max_ms": ms_of_ns(self.max_ns.load(Ordering::Relaxed)),
            "items": self.items.load(Ordering::Relaxed),
        })
    }

    /// The occurrences and the items totalled so far: a span's own counts of its steps. Zero when no sink is open.
    pub fn totals(&self) -> (u64, u64) {
        (self.count.load(Ordering::Relaxed), self.items.load(Ordering::Relaxed))
    }

    /// The thread time spent since `mark`, for a span that sums its steps (see [`Span::attach_since`]). Zero when no sink
    /// is open.
    pub fn spent_since(&self, mark: Mark) -> Duration {
        Duration::from_nanos(self.total_ns.load(Ordering::Relaxed).saturating_sub(mark.total_ns))
    }

    /// The count, total and items since `mark`. No maximum: the tally keeps one for the whole process, which may be an
    /// occurrence from before the span began (a write made by an earlier pass, say).
    fn since(&self, mark: Mark) -> Value {
        json!({
            "count": self.count.load(Ordering::Relaxed).saturating_sub(mark.count),
            "total_ms": ms_of_ns(self.total_ns.load(Ordering::Relaxed).saturating_sub(mark.total_ns)),
            "items": self.items.load(Ordering::Relaxed).saturating_sub(mark.items),
        })
    }
}

/// One occurrence of a [`Tally`]'s step, from [`Tally::timer`]: its time is added when it is dropped.
pub struct Timer<'a> {
    tally: &'a Tally,
    start: Option<Instant>,
    items: u64,
}

impl Timer<'_> {
    /// Items this occurrence processed.
    pub fn items(&mut self, n: u64) {
        self.items = n;
    }
}

impl Drop for Timer<'_> {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            self.tally.add(start.elapsed(), self.items);
        }
    }
}

/// Runs `f` with the records the calling thread makes taken into memory instead of the sink: they are returned in the
/// order they closed. Other threads' records (other tests', running at the same time) go to the sink as usual, so a
/// capture holds its own thread's records and nothing else. Captures run one at a time (the sink itself is set once per
/// process, so a test captures instead of opening one).
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<Value>) {
    let _alone = lock(&CAPTURE_LOCK);
    lock(&CAPTURED).clear();
    let was_enabled = ENABLED.swap(true, Ordering::Relaxed);
    let was_here = CAPTURE_HERE.with(|here| here.replace(true));
    let restore = RestoreCapture { was_enabled, was_here };
    let out = f();
    drop(restore);
    let records = std::mem::take(&mut *lock(&CAPTURED));
    (out, records)
}

/// The capture flags as they were before a capture, put back when it ends: by its return, or by a panic in its closure.
struct RestoreCapture {
    was_enabled: bool,
    was_here: bool,
}

impl Drop for RestoreCapture {
    fn drop(&mut self) {
        CAPTURE_HERE.with(|here| here.set(self.was_here));
        ENABLED.store(self.was_enabled, Ordering::Relaxed);
    }
}

/// The table of a perf file: for each session in it, the steps by name (count, total and self time, their share of the
/// session's top-level time, items and bytes with their rates), the pipelines' verdicts, and the scan's post-walk time.
/// Lines that aren't records (a torn last line, a line another version wrote) are skipped.
pub fn summarize(path: &Path) -> io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    Ok(summarize_text(&text))
}

/// The table of one session of a perf file (`session` is its id, as [`session_id`] gives it for this process).
pub fn summarize_session(path: &Path, session: u64) -> io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    let mut sessions = parse_sessions(&text);
    Ok(match sessions.remove(&session) {
        Some(records) => session_table(session, &records),
        None => "no perf records for this run\n".to_string(),
    })
}

/// [`summarize`] of a file's text.
pub fn summarize_text(text: &str) -> String {
    let sessions = parse_sessions(text);
    if sessions.is_empty() {
        return "no perf records\n".to_string();
    }
    let mut out = String::new();
    for (session, records) in &sessions {
        out.push_str(&session_table(*session, records));
        out.push('\n');
    }
    out
}

/// The records of a file's text, by session. Lines that aren't records are skipped.
fn parse_sessions(text: &str) -> BTreeMap<u64, Vec<Value>> {
    let mut sessions: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
    for line in text.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line)
            && let Some(session) = value.get("session").and_then(Value::as_u64)
        {
            sessions.entry(session).or_default().push(value);
        }
    }
    sessions
}

/// What the summary keeps of a step's records: their count, time, items and bytes.
#[derive(Default)]
struct Agg {
    count: u64,
    dur_ms: f64,
    self_ms: f64,
    items: u64,
    bytes: u64,
    failed: u64,
}

fn session_table(session: u64, records: &[Value]) -> String {
    let num = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let int = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0);
    let pid = records.first().map_or(0, |v| int(v, "pid"));
    // The session's wall time: from the first record opened to the last one closed. A span still open when the file was
    // read is not written yet, so it is not in the records, and it does not count.
    let steps = || records.iter().filter(|v| v.get("name").and_then(Value::as_str) != Some("session"));
    let start = steps().map(|v| num(v, "t_ms")).fold(f64::INFINITY, f64::min);
    let end = steps().map(|v| num(v, "t_ms") + num(v, "dur_ms")).fold(f64::NEG_INFINITY, f64::max);
    let top_ms = if end > start { end - start } else { 0.0 };
    let mut by_name: BTreeMap<String, Agg> = BTreeMap::new();
    for v in records {
        let name = v.get("name").and_then(Value::as_str).unwrap_or("?");
        if name == "session" {
            continue;
        }
        let agg = by_name.entry(name.to_string()).or_default();
        agg.count += 1;
        agg.dur_ms += num(v, "dur_ms");
        agg.self_ms += num(v, "self_ms");
        agg.items += int(v, "items");
        agg.bytes += int(v, "bytes");
        agg.failed += int(v, "failed");
    }
    let mut rows: Vec<(String, Agg)> = by_name.into_iter().collect();
    rows.sort_by(|a, b| b.1.dur_ms.total_cmp(&a.1.dur_ms));
    let mut out = format!("session {session} (pid {pid}): {} records, wall {:.1} s\n", records.len(), top_ms / 1e3);
    out.push_str(&format!(
        "{:<28} {:>6} {:>12} {:>12} {:>7} {:>10} {:>10} {:>12} {:>8}\n",
        "step", "count", "total s", "self s", "share", "items", "items/s", "bytes/s", "failed"
    ));
    for (name, agg) in &rows {
        let share = if top_ms > 0.0 { 100.0 * agg.dur_ms / top_ms } else { 0.0 };
        let secs = agg.dur_ms / 1e3;
        let per = |n: u64| if secs > 0.0 { pg_model::num::size(n) / secs } else { 0.0 };
        out.push_str(&format!(
            "{:<28} {:>6} {:>12.3} {:>12.3} {:>6.1}% {:>10} {:>10.0} {:>12.0} {:>8}\n",
            name,
            agg.count,
            secs,
            agg.self_ms / 1e3,
            share,
            agg.items,
            per(agg.items),
            per(agg.bytes),
            agg.failed
        ));
    }
    let total = |name: &str| {
        records
            .iter()
            .filter(|v| v.get("name").and_then(Value::as_str) == Some(name))
            .map(|v| num(v, "dur_ms"))
            .sum::<f64>()
    };
    let (phase, walk) = (total("phase.scan"), total("scan.walk"));
    if phase > 0.0 {
        out.push_str(&format!(
            "scan: {:.3} s of its phase is outside the walk loop (post-walk: pairing, missing marks, finish)\n",
            (phase - walk).max(0.0) / 1e3
        ));
    }
    // A pipeline's record sits under its pass's phase: its verdict is labelled with the nearest `pass.` span above it.
    let up: BTreeMap<u64, (&str, Option<u64>)> = records
        .iter()
        .filter_map(|v| {
            Some((v.get("id")?.as_u64()?, (v.get("name")?.as_str()?, v.get("parent").and_then(Value::as_u64))))
        })
        .collect();
    for v in records.iter().filter(|v| v.get("name").and_then(Value::as_str) == Some("pipeline")) {
        let pass = v.get("parent").and_then(Value::as_u64).and_then(|p| pass_above(&up, p)).unwrap_or("pipeline");
        let wall = v.get("dur_ms").and_then(Value::as_f64).unwrap_or(0.0);
        out.push_str(&format!("verdict {pass} ({:.1} s): {}\n", wall / 1e3, verdict(v)));
    }
    out
}

/// The name of the nearest `pass.` span at or above span `id`, by the records' parents (`up`: id to name and parent).
fn pass_above<'a>(up: &BTreeMap<u64, (&'a str, Option<u64>)>, mut id: u64) -> Option<&'a str> {
    loop {
        let (name, parent) = up.get(&id)?;
        if name.starts_with("pass.") {
            return Some(name);
        }
        id = (*parent)?;
    }
}

/// The bottleneck a pipeline's stage shares point to. The shares are percentages of the pass's wall time, and the
/// pauses within it are in the record's fields.
fn verdict(v: &Value) -> String {
    let f = |key: &str| v.get("fields").and_then(|f| f.get(key)).and_then(Value::as_f64).unwrap_or(0.0);
    let wall = v.get("dur_ms").and_then(Value::as_f64).unwrap_or(0.0);
    let paused = f("paused_ms");
    if wall > 0.0 && paused > 0.2 * wall {
        return format!("paused for {:.0}% of its wall time", 100.0 * paused / wall);
    }
    if f("workers_busy_pct") >= 60.0 {
        "CPU-bound: the workers are busy".to_string()
    } else if f("readers_busy_pct") >= 60.0 {
        "read-bound: the readers are busy".to_string()
    } else if f("sink_busy_pct").max(f("writer_busy_pct")) >= 60.0 {
        "writer-bound: the sink or the writer is busy".to_string()
    } else {
        "waiting: no stage is busy for 60% of the pass".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_with_no_sink_is_inert() {
        // Nothing is opened here (the test process may hold a capture of another test, which is serialised): this checks
        // only that a span built while disabled holds nothing.
        let (_, records) = capture(|| ());
        assert!(records.is_empty());
        let s = Span { live: None, _not_send: PhantomData };
        drop(s);
    }

    #[test]
    fn nested_spans_carry_their_parent_and_their_self_time() {
        let (_, records) = capture(|| {
            let mut outer = span("outer");
            {
                let mut inner = span("inner");
                inner.items(3);
                std::thread::sleep(Duration::from_millis(2));
            }
            outer.bytes(10);
        });
        let inner = records.iter().find(|v| v["name"] == "inner").expect("inner recorded");
        let outer = records.iter().find(|v| v["name"] == "outer").expect("outer recorded");
        assert_eq!(inner["parent"], outer["id"]);
        assert_eq!(outer["parent"], Value::Null);
        assert_eq!(inner["items"], 3);
        assert_eq!(outer["bytes"], 10);
        let (dur, self_ms) = (outer["dur_ms"].as_f64().unwrap_or(0.0), outer["self_ms"].as_f64().unwrap_or(0.0));
        assert!(self_ms <= dur + 1e-9, "self {self_ms} within duration {dur}");
    }

    #[test]
    fn a_tally_totals_its_threads_and_attaches_to_its_span() {
        static TALLY: Tally = Tally::new("work");
        let (_, records) = capture(|| {
            let mut s = span("pass");
            std::thread::scope(|scope| {
                for _ in 0..3 {
                    scope.spawn(|| {
                        let mut t = TALLY.timer();
                        t.items(5);
                    });
                }
            });
            s.attach(&TALLY);
        });
        let pass = records.iter().find(|v| v["name"] == "pass").expect("pass recorded");
        assert_eq!(pass["fields"]["work"]["count"], 3);
        assert_eq!(pass["fields"]["work"]["items"], 15);
    }

    #[test]
    fn a_measured_duration_is_a_child_of_the_open_span() {
        let (_, records) = capture(|| {
            let _outer = span("outer");
            record("measured", Duration::from_millis(4), &[("join_ms", json!(4.0))]);
        });
        let measured = records.iter().find(|v| v["name"] == "measured").expect("recorded");
        let outer = records.iter().find(|v| v["name"] == "outer").expect("outer recorded");
        assert_eq!(measured["parent"], outer["id"]);
        assert_eq!(measured["fields"]["join_ms"], 4.0);
        assert!(outer["self_ms"].as_f64().unwrap_or(f64::MAX) <= outer["dur_ms"].as_f64().unwrap_or(0.0));
    }

    #[test]
    fn the_summary_totals_steps_and_skips_torn_lines() {
        let text = concat!(
            "{\"session\":7,\"pid\":1,\"t_ms\":0.0,\"id\":0,\"name\":\"session\",\"parent\":null,\"dur_ms\":0.0,\"self_ms\":0.0,\"items\":0,\"bytes\":0,\"failed\":0,\"fields\":{}}\n",
            "{\"session\":7,\"pid\":1,\"t_ms\":1.0,\"id\":1,\"name\":\"cmd.import\",\"parent\":null,\"dur_ms\":2000.0,\"self_ms\":500.0,\"items\":0,\"bytes\":0,\"failed\":0,\"fields\":{}}\n",
            "{\"session\":7,\"pid\":1,\"t_ms\":2.0,\"id\":2,\"name\":\"pass.import\",\"parent\":1,\"dur_ms\":1500.0,\"self_ms\":1500.0,\"items\":300,\"bytes\":0,\"failed\":0,\"fields\":{}}\n",
            "{\"session\":7,\"pid\":1,\"t_ms\":3.0,\"id\":3,\"name\":\"pipeline\",\"parent\":2,\"dur_ms\":1400.0,\"self_ms\":1400.0,\"items\":300,\"bytes\":0,\"failed\":0,\"fields\":{\"pass\":\"import\",\"workers_busy_pct\":90.0}}\n",
            "{\"session\":7,\"pid\":1,\"t_ms\":4.0,\"id\":4,\"name\":\"pass.im",
        );
        let out = summarize_text(text);
        assert!(out.contains("pass.import"), "{out}");
        assert!(out.contains("CPU-bound"), "{out}");
        assert!(out.contains("wall 2.0 s"), "{out}");
    }
}
