# Yandex result images: a per-server fetch limit instead of a global `FETCHERS = 4`

## Context

The Yandex screen prefetches each search's result files with a global `FETCHERS = 4` threads
(`crates/app/src/yandex/images.rs:24`, started by `Images::prefetch`). Results sit on many different servers, so a
global 4 under-uses the network when they are spread out and promises nothing per server. The limit becomes a
per-server constant, and it must hold for every request in flight: prefetch, on-demand, each redirect hop, and across
searches (a stopped search's downloads outlive its stop).

Decisions: **6 per server**, **no global cap**, **server = host + port** (confirmed by the user, host+port is my
addition: a server is an endpoint). Design rule from the user: the most proper option, churn across crates accepted.

Why `pg_yandex` changes, not only `Images` (ureq 3.5.2 source checked): ureq has no per-redirect hook (`run.rs`
follows redirects inside one call; `get_uri()`/`get_redirect_history()` are known only afterwards), but
`max_redirects(0)` makes it return the 3xx response itself (`config.rs:261-262,329`; `agent()` already sets
`http_status_as_error(false)`). So `Fetcher::download` follows redirects itself and takes a slot per hop, and the
limit lives next to the network code.

## Scope: what the request needs, and what this plan adds

- **Needed for a correct per-server limit:** the `Server` key and `ResultUrl::parse`; the gate; a scheduler that
  picks the next *eligible* result (a full server must not block the queue); a process-wide gate with a wake path
  to other searches' `Images`; on-demand `get` going through the queue; the decode and memory bounds that
  `FETCHERS` also provided; `get`'s inline fallback.
- **Optional, added by the "most proper" rule; the user should know:** (1) **per-hop redirect metering**, the
  biggest jump: `max_redirects(0)`, a manual loop, **redirects cut from ureq's 10 to 5**, a 3xx without `Location`
  changing from a transport error to a `Status` error, drain-to-EOF. The cheaper variant meters only the URL's own
  server and documents that redirect targets are not metered. (2) The agent's idle-pool tuning (performance only).
  (3) Building the security plan's `ResultUrl` and the `NotWebAddress` reason now. (4) Robustness against thread
  spawn failure and panics (`Kick::pause`, the job-in-hand cell, the panic guard).

## Design

### A. `crates/yandex` (`pg_yandex`)

New files `result_url.rs` (not `url.rs`: a module named `url` makes `use url::Url` ambiguous with the external
crate) and `gate.rs`; `fetch.rs`, `http.rs`, `lib.rs`, `Cargo.toml` change.

**`ResultUrl` and `Server`** (`result_url.rs`). `docs/plans/SECURITY_HARDENING_FIXES.md` §3 (not yet implemented)
already calls for a `ResultUrl` in pg-yandex ("http or https, a host, no userinfo", `Fetcher::download` takes it);
this builds that type so the two plans do not each invent one.

- Private fields; the only constructor is `ResultUrl::parse(&str) -> Option<ResultUrl>`, using the `url` crate once:
  http/https only, a host, **no userinfo**, host lowercased and IDNA-normalised, port explicit or the scheme's
  default; anything else → `None`.
- `server() -> &Server`, `as_str()` (normalised; what every request uses, including the first hop, so the gate's key
  is the host ureq connects to: ureq parses with `http::Uri`, which differs from `url` on `\`, IDN and fragments),
  `original() -> &str` (as given: the replay's `downloads.tsv` and the test fakes look files up by it, because
  `as_str()` normalises, e.g. `https://a.example` gains a `/`), `join(&self, location: &str) -> Option<ResultUrl>`.
- `Server`: `Ord + Clone + Debug` (host, port).
- Dependency: `url` 2.5.9 is already in `Cargo.lock` (via `cookie_store` and ureq's cookies); add `url = "2"` to the
  workspace dependencies and `url.workspace = true` to `crates/yandex/Cargo.toml`. `deny.toml` bans nothing about
  it (it bans `ureq` outside `pg-yandex`); `tools/crate-layers.txt` is unchanged.
- Security plan §3 is edited in the same change: its `.max_redirects(5)` line says downloads follow redirects by
  hand with `MAX_REDIRECTS = 5`; "built by the parser" means the parser calls `ResultUrl::parse`. Its other items
  (public-addresses-only resolver, `.proxy(None)`, `https_only`, the parser building `ResultUrl` into `Hit`) stay
  there; the manual loop is compatible (each hop is a fresh request through that resolver). Its "four at a time" →
  "six at a time from each server".

**`Gate`, `Permit`, `Kick`** (`gate.rs`). `pub const PER_SERVER: usize = 6;` is the constant.

- `Gate::new(limit) -> Arc<Gate>`. State: `Mutex<{ counts: BTreeMap<Server, usize>, waiters: BTreeMap<Server,
  usize>, wakers: Vec<Weak<Kick>> }>` + `Condvar`. Every lock is `lock().unwrap_or_else(PoisonError::into_inner)`
  (`Permit::drop` and `Kick::kick` run during unwinds); a map entry is removed when its count reaches 0.
- `try_acquire(self: &Arc<Self>, &Server) -> Option<Permit>`: non-blocking; refuses at the limit **and while the
  server has waiters** (a redirect hop already did work and gave up its previous slot; the scheduler must not take
  every freed slot ahead of it).
- `acquire(self: &Arc<Self>, &Server, cancel: &AtomicBool) -> Result<Permit, Cancelled>`: blocks on the condvar,
  registered in `waiters` through an RAII guard (a panic cannot leave a phantom waiter), tests `cancel` **under the
  gate lock** before each wait. When a waiter leaves (acquired or cancelled), and the server then has no waiters
  and `count < limit`, the gate kicks the wakers (the scheduler may have been refused by the waiter's presence, and
  no later release would wake it).
- `wake()`: takes the gate lock, **then** `notify_all` (a waiter that has just seen `cancel == false` cannot miss
  it). `Images::stop` calls it after setting `cancel`.
- `Permit` (owns `Arc<Gate>` + `Server`; `server()`, `gate()`): on drop it decrements, `notify_all`s the condvar,
  and **after releasing the gate lock** kicks every live waker, pruning dead `Weak`s in the same pass.
- `subscribe(&self, Weak<Kick>)`: registers a scheduler's wake handle (`Images::new`, through `Fetch::gate()`).
  `in_flight(&Server) -> usize` for tests.
- `Kick { state: Mutex<{ flag, closed }>, cv }`: `kick()` sets the flag and notifies and touches nothing else (safe
  from `Permit::drop` anywhere, even under the scheduler's table lock); `wait()` blocks until kicked or closed and
  clears the flag; `pause(d)` blocks up to `d` or until `close()` **ignoring the flag** (a backoff built on `wait`
  would be released at once by the failed spawn's own permit drop); `close()` sets `closed` and notifies.
- **One gate for the process, not per search**: `client()` builds a `Fetcher` per search (`app/src/yandex.rs:82-85`)
  and a stopped search's downloads outlive its stop (cancel is checked only between body reads; the headers phase
  runs up to 30 s, `fetch.rs:36-38`). `Yandex::new` creates one `Arc<Gate>` and hands it to `client(gate)` /
  `replay(dir, gate)`.

**`Fetcher`** (`fetch.rs`, `http.rs`)

- `Fetcher::new(gate: Arc<Gate>)`; `pub fn gate(&self) -> &Arc<Gate>`. **`impl Default for Fetcher` is deleted**
  (its only caller is `yandex.rs:84`).
- Its **own agent**: `max_idle_connections_per_host(PER_SERVER)` and `max_idle_connections(IDLE_TOTAL)`, `const
  IDLE_TOTAL: usize = 4 * PER_SERVER`. ureq's defaults are 3 per host and 10 in total (`config.rs:1088-1089`), so six
  concurrent fetches would reopen connections (TLS included) every wave, and with more than two hosts the total
  would evict them. Both are agent-level only ("no effect when used per-request", `config.rs:699,709`), so
  `http.rs` gains `agent_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope>` (public:
  `lib.rs:631-635`; what `agent()` builds from today) and `Fetcher::new` finishes it. The search client keeps
  `http::agent()`.
- **`download(&self, url: &ResultUrl, first: Permit, cancel: &AtomicBool) -> Result<Download, YandexError>`**
  follows redirects itself. `first` must be a permit for `url.server()` **of this fetcher's gate** (`Arc::ptr_eq`;
  a permit of another gate would let a same-server chain run unmetered), else a `Transport` error (a caller bug,
  not a panic). Per hop:
  1. `cancel` is checked at the top of every hop (a same-server hop never calls `acquire`, so a stopped download
     could otherwise follow `MAX_REDIRECTS` hops of up to 30 s each) and while draining a 3xx body.
  2. Request `as_str()` with `max_redirects(0)` set **in `download` only** (`transfer_timeouts` is shared with the
     upload in `client.rs`) plus the existing `transfer_timeouts`. The agent's cookie jar works per hop (stored and
     attached per request, `run.rs:198-210,299`).
  3. ureq's own redirect rules, copied: a redirect is any 3xx **except 304** (`ureq-proto` `client/mod.rs:314-318`);
     the target is the **last** `Location` header (`recvresp.rs:70-76`), resolved with `ResultUrl::join` (no host,
     userinfo or other scheme → `Transport` error). `const MAX_REDIRECTS: u32 = 5` (the security plan's decision;
     ureq's default was 10): at most `MAX_REDIRECTS + 1` requests (ureq's meaning, `run.rs:186`), then a `Transport`
     error with ureq's "too many redirects" wording. A 3xx without `Location` is a `Status` error (ureq's path raised
     `NoLocationHeader`, a transport error: a stated change of kind).
  4. A redirect to the **same server** keeps its permit. A redirect to a **different** server: read the 3xx body to
     EOF up to 16 KiB **including the final `Ok(0)`** (ureq returns a connection to its pool only on EOF,
     `BodyHandler::ended`; `run.rs:764-771` drains the same way), **drop the `Response`**, **drop the previous
     permit**, **then** `acquire` the new server (cancel-aware). A hop never holds one slot while waiting for
     another, so there is no hold-and-wait deadlock, and a server never sees more than its limit open.
  5. `Cancelled` from `acquire` becomes `YandexError::Transport("the download was cut short at close")`, which the
     existing rule in `impl Fetch for Fetcher` (`images.rs:38`: any error while `cancel` is set →
     `SourceError::Closed`) maps, so `settle` puts the slot back to `Queued` like a body cut short.
  6. The final hop's body is read as today (status, content-type check, 100 MiB cap, cancel between reads). The
     **permit is dropped as soon as the body is fully read**, before sniffing and decoding.
- `lib.rs` re-exports `ResultUrl`, `Server`, `Gate`, `Permit`, `Kick`, `Cancelled`, `PER_SERVER`.
- **Workspace lints apply** (`Cargo.toml` `[workspace.lints]`, `clippy.toml`): every `pub` item and field is
  documented (`missing_docs`); `pub` only if another crate names it (`unreachable_pub`: `Gate`, `Permit`, `Kick`,
  `Server`, `ResultUrl` are used by `pg_app`, the rest `pub(crate)`); no `unwrap`/`expect`/indexing/`as`/`panic!` in
  production code; `BTreeMap`/`BTreeSet`, never `HashMap`/`HashSet`; no printing. Locks through small `lock()`
  helpers like `images.rs:122`.

### B. `crates/app/src/yandex` (`pg_app`)

`images.rs` is the scheduler (order, promotion, decode bound); `yandex.rs`, `replay.rs` follow the new signatures.

**`Fetch` and the replay**

- `trait Fetch: Send + Sync { fn gate(&self) -> &Arc<Gate>; fn fetch(&self, url: &ResultUrl, permit: Permit, cancel:
  &AtomicBool) -> Result<Download, SourceError>; }` (object-safe). `impl Fetch for Fetcher` forwards. The doc of the
  trait is updated.
- The replay's file-serving half becomes its own type `ReplayFiles { recording, gate }` (built by `replay(dir,
  gate)`, looked up by `url.original()`, dropping the permit at once), exported with `pub use replay::{Recording,
  Replay, ReplayFiles}` (`yandex.rs:39`). `Replay` (the `Source` half) keeps `Replay::new(recording)` with no gate.
  `client(gate)` and `replay(dir, gate)` are public in `pg_app::yandex` but used only by `Yandex::new` and the replay
  test (grep: the desktop uses only `YandexSource`), so their signatures change freely.

**`Images` state** (`Table`; `fetchers` is removed, nothing else reads it)

- `Slot { url: ResultUrl, state: State }` (the slot `State` enum `Queued | Fetching | Ready | Dead` is unchanged;
  `Hit` keeps carrying the `String`); `queue: VecDeque<u64>`; `dead`, `sized`, `stopped` as now; **`undecoded:
  usize`** (bodies downloaded and not yet decoded); **`scheduler: SchedulerState { NotStarted, Running, Failed }`**.
- `Images` fields: `kick: Arc<Kick>` (subscribed to the gate in `new`), `decoders: Semaphore` (its own
  `Mutex<usize>` + `Condvar`), `spawn: fn(ThreadKind, &str, Job) -> io::Result<()>` (test seam, default
  `thread::Builder`; `ThreadKind { Scheduler, Fetch }` so a test can fail one and not the other, like `spawn_with`,
  `yandex.rs:433`), `backoff: Duration` (250 ms; a test sets a few ms).

**Scheduling in `Images`**

- `next_queued(&self, gate) -> Option<Job>` (`Job { id, url: ResultUrl, permit: Permit }`): under the table lock,
  `None` when stopped **or `undecoded >= DECODE_BACKLOG`**; otherwise the first id in `queue` whose slot is `Queued`
  **and** whose server `gate.try_acquire` grants (a server refused earlier in the same scan is not asked again, so a
  scan costs one gate call per distinct server); remove it from `queue`, mark `Fetching`, return it. A full server's
  entries keep their place (no head-of-line blocking across servers; order within a server is the queue's). Entries
  whose slot is no longer `Queued` are dropped as today. Lock order: table → gate → kick; `Permit::drop` takes the
  gate lock, releases it, then kicks, so a permit dropped under the table lock cannot deadlock.
- `pump(self: &Arc<Self>, want: Option<u64>) -> Pumped` (`Pumped { started: usize, spawn_failed: bool, want_job:
  Option<Job> }`): loop { `next_queued`; keep the job in an `Arc<Mutex<Option<Job>>>` **outside** the closure
  (`spawn` consumes the closure, and a permit moved into it would be dropped by a failed spawn); spawn `yandex image
  {id}`, whose body `take()`s the job and runs `fetch_caught`; on spawn failure `take()` the job back: if it is
  `want`'s, return it in `want_job` (the caller fetches inline, holding its permit), otherwise put it back in
  `Queued` at the front of the queue, drop its permit, set `spawn_failed`, and stop }. A guard returns an unstarted
  job to `Queued` (permit dropped) if `pump` unwinds between `next_queued` and the spawn, so no slot stays
  `Fetching` with no thread.
- **The fetch thread**: `fetch(url, permit, cancel)` (the permit is released inside it, at the end of the body, which
  kicks) → on `Ok`: `undecoded += 1` → wait for a decoder slot (below) → `check` → `undecoded -= 1`, **kick the
  scheduler** (the backlog may have cleared) → `settle`. Fetch threads never pump.
- **Decode bound** (`FETCHERS = 4` also capped decodes and buffered bodies, and `Makers::with_maker` builds a new
  maker when none is idle, `makers.rs:22-26`): `const DECODERS: usize = 4;` concurrent `check`s through the
  semaphore, and `const DECODE_BACKLOG: usize = 2 * DECODERS;` bodies waiting or being decoded before the scheduler
  stops starting fetches. The decoder wait tests the `cancel` flag **under the semaphore's mutex** before each wait,
  and `stop` takes that mutex before `notify_all` (no lost wakeup); a cancelled waiter ends as
  `SourceError::Closed`.
- **The scheduler thread** (one per `Images`): `ensure_scheduler()` (called by `prefetch` and by `get`; returns
  nothing) does under the table lock `if stopped { return }` and `NotStarted → Running` (so no double start, none
  after `stop`; `Yandex::stop` can land between the search worker's `drop(s)` and `prefetch`, `yandex.rs:506-510`),
  then spawns `yandex scheduler` holding `Weak<Images>` + `Arc<Kick>`. Loop: `kick.wait()`; upgrade or exit; exit if
  stopped; `pump(None)` inside `pg_jobs::panics::catch` **per pass** (a panic is logged, the pass's job returns to
  `Queued`, the loop continues; **3 panics in a row end it as `Failed` for good**, `settled` notified; no respawn);
  if `spawn_failed`, `kick.pause(backoff)` (stop-aware via `close()`) before the next pass. It exits on `stop` or
  when the `Images` is dropped (`Drop for Images` closes the kick). Kicked by: `prefetch`, `get`'s promotion, any
  permit release or waiter exit (the gate), a decode ending, `stop`. If its own spawn fails the state is `Failed`.
- **`prefetch(results)`**: for each `(id, url)`: `ResultUrl::parse`; on `None` the slot is inserted `Dead`
  (`NotWebAddress`), the id pushed to `dead`, `changes.publish(())` called (as `settle` does) and it is not queued;
  otherwise inserted `Queued` and queued. Then `ensure_scheduler()` and a kick.
- **`get(id, url: &str)`**: `Ready` → return; `Dead` → error; stopped → `Closed`. A URL that does not parse is
  handled exactly like `prefetch` (a `Dead` slot, `dead`, `publish`, the error). Otherwise the result **joins the
  same queue at the front** (slot inserted if unknown, promoted if `Queued`), `ensure_scheduler()`, kick, then one
  wait rule with no case split: wait on `settled` with `wait_timeout(200 ms)`; on every wake, while its slot is still
  `Queued`, call `pump(Some(id))` itself and run `want_job` inline if the spawn failed; return when the slot is
  `Ready`/`Dead` or the images stop. So `get` never depends on the scheduler (not when it is `Failed`, nor when only
  fetch spawns fail), and two pumpers racing is harmless (`next_queued` and `try_acquire` are serialised by the
  table and gate locks). The separate `yandex fetch {id}` thread block is removed. `get_known` is unchanged.
- **`stop`**: sets `stopped` under the table lock and `cancel`, then `fetch.gate().wake()`, the decoder mutex +
  `notify_all`, `kick.close()`, `settled.notify_all()`. `Yandex::stop` still polls only `busy` (`yandex.rs:547-556`)
  and joins no thread: scheduler and fetch threads are not waited for (a comment says so); a `get` caller gets
  `Closed` at once.
- **Behaviour change, stated plainly:** an on-demand `get` now waits for a free slot on its server (behind up to 6
  running downloads, each bounded by the transfer timeouts) instead of opening a 7th connection at once. It goes
  next in the queue; the limit is never exceeded.

**Why a scheduler thread, and why the gate wakes it.** With one process-wide gate a stopped search can still hold all
6 slots of a server (headers phase, up to 30 s). The new search's `prefetch` then gets `try_acquire == None` for
every result and starts nothing; when the old downloads finish, nobody would pump the new `Images` (the old one is
stopped), so its results would stay `Queued` and any `get` would wait forever. The gate's release kick wakes every
live `Images`' scheduler, which also starts the next queued result the moment a redirect hop frees a slot.

### C. Qt provider pool: no change

The Yandex pool has 4 threads (`providers.cpp:157-161`) and `yandex_picture` (`providers.rs:99-100`) calls the
blocking `Images::get`. The grid asks for **one picture at a time, in row order** (`ResultTile.qml:62-67` loads
`tile.thumb` only when `tile.index <= loadTurn`; `settle_turn`, `desktop/src/yandex.rs:123`, advances only when the
row at the turn settles as `Ready` or `Error`, `ResultTile.qml:72-75`), so it parks at most one pool thread; the
preview's current and two neighbouring originals (`yandex.rs:444-449`) at most three more: exactly the pool, which
nothing else uses. The prefetch queue is in row order, so the row at the turn is normally already fetched or next.
**No desktop, QML or `providers.rs` change** except the comment at `providers.cpp:159-160` ("fetches on demand, so
four at a time"), which now says why 4: one grid picture plus the preview's three originals. Stated consequence:
stepping quickly through unfetched previews queues stale requests behind waits until a slot frees. (A gating on
`imageWidth` was considered and rejected: a tile with no size would never settle and would hold every later row.)

### D. New reason, comments, docs

- `UnexpectedReason::NotWebAddress` ("the address is not a web (http or https) address"): variant in
  `crates/model/src/yandex.rs`; matched in `crates/wording/src/lib.rs:391-403` and `crates/wording/src/message.rs`
  (`YandexNotWebAddress`); text in `crates/wording/locales/en/text.ftl` (only the `en` locale exists).
  `crates/jobs/src/source.rs` does **not** need it (its `FileTooLarge` is `io::ErrorKind`).
- Stale comments: module doc of `images.rs` ("a few at a time" → "a few at a time from each server"), `State`/`Table`
  docs, the `Fetch` and `get` docs ("runs on a thread of its own"), and the comments naming fetchers or the on-demand
  fallback at `app/src/yandex.rs:63, 165, 425, 466, 490, 539-541`.
- `docs/plans/SECURITY_HARDENING_FIXES.md` §3 edits (above). `docs/plans/SECURITY_FLOW_REVIEW.md:1017` ("four
  fetchers") is a historical finding and stays.

## Concurrency invariants (what each test below pins)

| Event                                                            | Who wakes whom                                                                               |
|------------------------------------------------------------------|----------------------------------------------------------------------------------------------|
| Permit dropped (download end, hop switch, spawn failure, unwind) | gate condvar (hop waiters) and every subscribed `Kick` (schedulers)                          |
| Hop waiter leaves with slots free and no waiters                 | every subscribed `Kick`                                                                      |
| Decode ends (`undecoded` falls)                                  | this `Images`' `Kick`                                                                        |
| `prefetch` / `get` promotion                                     | this `Images`' `Kick`                                                                        |
| `stop`                                                           | `Gate::wake` (lock then notify), decoder mutex + notify, `Kick::close`, `settled.notify_all` |
| `settle`                                                         | `settled.notify_all` (waiting `get`s); `changes.publish`                                     |

Lock order everywhere: table → gate → kick. Nothing blocks while holding the table lock.

## Tests (synthetic; no network)

Positive waits use channels with a timeout, never sleeps.

**`pg_yandex`**

- `ResultUrl::parse`/`Server`: case, default ports, explicit port, userinfo → `None`, IDN, `\`, fragments,
  non-http scheme / no host → `None`; `original()` vs `as_str()`; `join` (relative, absolute, protocol-relative).
- `Gate`: the limit holds; a permit drop frees and wakes a blocked `acquire`; `cancel` + `wake` ends a blocked
  `acquire` even when `wake` races the check (looped); a waiter blocks `try_acquire` on its server until it has its
  slot, and the wakers are kicked when it takes it with slots left; the waiter guard is balanced after a panic; a
  permit drop kicks subscribed wakers and prunes dead ones; a `Permit` dropped during `catch_unwind` is released;
  a poisoned lock does not abort a drop; `Kick::wait` clears its flag; `Kick::pause` is not released by a kick and
  is by `close()`; two `Fetcher`s built with one gate share one limit.
- `download` against loopback `TcpListener` servers (two listeners = two servers, 127.0.0.1 on different ports; the
  server holds the second connection open while the test checks `Gate::in_flight`), through a `#[cfg(test)]`
  constructor whose agent has **no proxy** (ureq reads `ALL_PROXY`/`HTTP_PROXY`, `config.rs:1073`) **and no
  public-addresses-only resolver** (127.0.0.1 would be refused once the security plan lands): absolute and relative
  `Location`; the last `Location` header wins; 300 followed, 304 not; hop 1's slot released and hop 2's taken; a
  same-server redirect keeps the count; a 3xx body drained to EOF (the connection is reused); `Location` without a
  host → error; `MAX_REDIRECTS` redirects ok, the next → error; 3xx without `Location` → `Status`; a permit for the
  wrong server and one of another gate → error; the permit is released when `download` errors or is cancelled;
  cancel stops a same-server redirect chain.

**`pg_app`** (`images.rs`, gating `Fetch` fakes, the spawner seam, `#[cfg(test)]` fault hook that makes `pump` panic
on its Nth call, and a fake whose `fetch` calls `gate().acquire` itself)

- peak in-flight on one server == 6 with 10 queued; all fetched once released; decodes never exceed 4; with slow
  decodes `undecoded` never exceeds `DECODE_BACKLOG` and the scheduler resumes when a decode ends.
- a saturated server does not hold up another server's result; order within a server follows the queue.
- on-demand respects the limit: a `get` for a 7th result of a full server waits, then is fetched next, ahead of the
  other queued ones.
- **cross-`Images` stall**: images B queued behind permits held by A's fetches; releasing them starts B's results
  (fails without the gate kick); a `get` on B then returns.
- failures: scheduler spawn fails → `get` still fetches; scheduler OK but every fetch spawn fails → `get` fetches
  inline; both fail → the same; a spawn failure hands the job and its permit back (permit count unchanged, slot back
  in `Queued`) and the scheduler backs off (few attempts in a bounded window with a few-ms backoff); an unwinding
  `pump` returns its unstarted job to `Queued`; the scheduler survives a panicking pass, and 3 in a row end it as
  `Failed` with `get` unaffected; `stop` ends hop waiters, a decode waiter (looped, racing its check) and a `get`
  caller; `ensure_scheduler` after `stop` starts nothing; the scheduler exits when its `Images` is dropped.
- a no-server URL (`ftp://…`, no host, userinfo) is dead once, in `dead`, announced on `changes`, by `prefetch` and
  by `get`.
- the replay finds a URL that normalises differently (`https://a.example`, a space) through `original()`;
  `Yandex::new` shares one gate across searches.
- Existing tests keep their assertions; their fakes change signature (`Counting`, the replay split, `Images::new`
  and `Replay::new` callers at `yandex.rs:84,94-95`, `replay.rs:188,205,236`, `images.rs:367`); all their URLs
  (`a.example`, `gone.example`, `i.example`) have servers.

## Implementation order (each step compiles and passes its own tests)

1. Workspace `url` dependency; `ResultUrl`/`Server` and their tests (additive).
2. `Gate`/`Permit`/`Kick` and their tests (additive, unused).
3. `http::agent_config()` (additive).
4. `UnexpectedReason::NotWebAddress` in model, wording, `text.ftl` (additive).
5. **One atomic change** (these cannot land separately): `Fetcher::new(gate)` and `download` with the redirect
   loop; the `Fetch` trait with `gate()` and a `Permit`; the replay split; `client(gate)`/`replay(dir, gate)`;
   `Yandex::new` creating the shared gate; `Images`' scheduler, `pump`, `next_queued`, `get`, `stop`, the decode
   semaphore and backlog; the stale comments; the `providers.cpp` comment; the security-plan edits.

## Verification

Native Windows, checks batched at the end (no full static Qt build): `cargo check`, `cargo test`, `cargo clippy`
for `pg_yandex`, `pg_app`, `pg_model`, `pg_wording`; the replay-based tests in `crates/app/src/yandex/replay.rs`
exercise `prefetch` end to end; `python tools/boundaries.py` and `just lint` (clippy, `cargo deny`, cargo-machete:
the new `url` dependency is used). `Cargo.lock` gains `url` under `pg-yandex`. The desktop crate changes only a C++
comment. Any app run uses scratch `PG_HOME`/`PG_CATALOG_DIR`/`PG_PREFS`, never `D:/PhotoGallery`.

## Consequences to flag, not change

- **No global cap** (the user's choice): threads and memory scale with distinct servers (6 per server). A body can
  be up to 100 MiB, so a 100-result search on 100 hosts could in theory hold ~10 GB in flight. Decodes stay at 4 and
  downloaded-but-undecoded bodies under `DECODE_BACKLOG`, so a single-server search buffers at most that plus 6 in
  flight. A second, global gate-level cap can be added later without redesign.
- The limit is on in-flight requests; idle pooled sockets are not gated, and with a proxy in the environment it is
  per origin, not per socket.
- Redirects 10 → 5 and the 3xx-without-`Location` error kind (see Scope).

## Not changed

The search (paced, single thread), the save's logic (it keeps the blocking `get`), `plausibly_image`, the body cap
and the timeouts, the desktop and QML code.
