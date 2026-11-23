# Plan: retire or tighten the non-empty allowlists

> Line numbers were refreshed on 2026-11-22 against a working tree that is still changing (the uncommitted
> `missing_docs` pass). Where a cite is not refreshed it is a snapshot: find the site by its quoted text.

## Context

An audit of the repo's allowlists (every non-empty one except `tools/anyhow-allow.txt`, which Phase 7 of the consistency
program already retires) found that most of them paper over something that can be refactored: a test-only dependency that
drags in an unmaintained crate, a crate that links codecs it should not, an environment write in the wrong crate, forks
that could be upstreamed, dead or stale entries, and per-site lint exceptions that are really owed refactors. This plan
removes each entry that has a proper refactor, tightens the ones that stay, and records why the rest stay. Churn is not a
concern; correctness and the intended layering are.

In scope: `PUBLIC_API_EXEMPT`, `ENV_READERS`, `REEXPORT_BOUNDARIES` (`tools/boundaries.py`); `advisories.ignore`, `bans`
wrappers, `sources.allow-git`, `licenses.allow` (`deny.toml`); `SYSTEM_DLL` (`tools/build_static_ffmpeg.py`); bindgen
`TYPES`/`VARS` (`crates/video/build.rs`); the `testkit` re-exports; the tooling users of `rusqlite`; the per-site
`#[expect]` exceptions; and the tooling and CI exceptions (Python lint ignores and `noqa` comments, the test-input list,
one CI waiver). Out of scope because empty: `tools/modgraph-allow.txt`, `PLATFORM_GLUE`,
`check_feature_coverage.ALLOWED`.

End state: an empty `REEXPORT_BOUNDARIES`, no `PUBLIC_API_EXEMPT`, `ENV_READERS` reduced to the registry reader's two
functions, an exact-name `SYSTEM_DLL` policy that fails the build, a bindgen allowlist that checks itself, no advisory
ignores, no `rusqlite` or codec wrapper for a crate that touches another crate's database or codecs, an `allow-git` that
lists only forks carrying changes upstream has not taken, the owed lint refactors done (including the front ends' printing,
the desktop's `exit` and the `unsafe_code` sites that need not exist), single-use licenses as per-crate exceptions, every
Rust workspace under the same deny, clippy and fmt gates, and the tooling and CI exceptions tightened or justified.

## Ground rules

- Make edits with the Edit/Write tools, not scripts.
- Batch `cargo check`/`cargo test` until a phase's edits are all in; do not validate after each edit. (Phase B's one-off
  printer run is the exception because it produces data.)
- Compile and check natively on Windows (PowerShell, with `PG_QT_DIR` set: `tools/check_qt.py:18` reads it, and the `_qt`
  prerequisite of `just lint`, `check`, `test` and `modgraph` fails without it). Linux checks go through
  `just check-linux` (Docker), not WSL. Never run the static Qt or static FFmpeg builds to verify a change.
- No commits or PRs unless asked. Each phase is one reviewable commit; Phase K is the exception (one commit per lint group,
  validated after each group: a batch of edits, not each edit).
- `missing_docs = "warn"` is on and `-D warnings` makes it an error: every new `pub` item needs a doc comment, and for a
  `pub struct`/`enum` that means each `pub` field and each variant too. This covers the plan's new items (`Request`,
  `Pass` and their fields; `jpeg::size`; `pub unsafe fn prefer_pulseaudio_if_pipewire_fails`; `pg_codec::Error::Imperfect`
  and `NothingLeft`; the `pub mod` and fixtures D makes public; every new `pg-model` type, field, variant and method in
  Phase H, so `cache.rs` keeps its per-method doc comments). Items that move keep the docs they have.
- Line numbers are a snapshot of a working tree that keeps changing. Where a step cites a file the in-progress work is
  editing, find the place by the quoted text and re-read the file first.
- Before each phase: run `git status` and `python tools/boundaries.py`, and treat "boundaries passes" as "no failure beyond
  the baseline you just measured". Every phase ends with `python tools/boundaries.py` and `just test-tools` (all
  `tools/test_*.py` files via `unittest discover`: ten today, eleven once Phase E adds its test), in addition to its own
  "Done when" line. `just deny` is a full `cargo deny check`; run it after A, B, D, G and J (each edits `deny.toml` or the
  dependency graph).

## Status and open work

Phases A to F, H to J, and K's lint groups K.1, K.2, K.5, K.8 to K.11 are complete and committed (the phases are in
`284d0505`, `8e96dbd4`, `1849f297`, `5d0b6765`, `b3eb0c31`, `e1c2b046` and the commits after them). Their sections were
removed from this plan; the history has them. Phase G is partly done (G0, the turbojpeg measurement, and the allocation
measurement). What remains:

- **G1, rusqlite flags:** deferred until upstream ships the statement-cache fix. Then move the two flags to
  `.cargo/config.toml` and drop `photo-gallery-defaults`.
- **G2, the rest of the measurements (skipped at your request):** rusqlite's cache fix and flags, dhat-rs, parajpeg,
  cxx-qt; the GIF, TIFF, zune-jpeg and thumbhash paths need samples that exercise them; Linux `just perf`.
- **G3, fork retirements:** none yet. Each needs its measurement and your decision.
- **L.9, MD060:** the table-style rule stays disabled (`.markdownlint.jsonc`). Enabling it, and pinning a style, is
  undecided. The rest of the tooling and CI work is done.
- **K.3, benches:** six `clippy::panic` expectations remain: five in `crates/bench/benches/ir/catalog.rs` and one in
  `crates/bench/benches/ir/linux.rs`. Return `Result` from the bench helpers instead.
- **K.4, strings and indexing:** `crates/desktop/src/qml_text.rs:94` and `crates/metadata/src/lib.rs:78` still have
  `string_slice` expectations (fix with `split_once`, `strip_prefix` or `str::get`). `crates/metadata/src/container.rs:21`
  (`nul_terminated`, a `const fn`) still has an `indexing_slicing` expectation.
- **K.6, singles:** `crates/codec/src/rewrite/op.rs:61` still has `type_complexity` on `Op::frame`. Name the type.
- **K.7, `unreachable_pub` in the desktop:** twenty expectations remain in `crates/desktop/src/*/state.rs`. The plan asked
  to try one `bridges` module for all of the desktop's, which means moving the state modules too. Needs your decision.
- **Desktop `print_stderr`:** `crates/desktop/src/trace.rs:24` has an expectation the plan does not list (the `PG_TRACE=1`
  console output). Keep it with its reason, or confirm.
- **Fork port follow-ups:** `just check-linux` after the `fast_image_resize` pin move; NEON and WebAssembly backends of the
  port are not compiled anywhere.

---

## Concrete designs (the former sketches, written against the code as read)

Each design below was written from the current source (Phase 7 in progress). Code is shown as it should read after the
change; "edit list" items are the exact sites. None of it has been compiled. Names were checked for collisions in the
crate they go into (`Renewal`, `Start` in `scan`; `Request` in `codec`: free; `Pass` in `jobs`: no type of that name, and
the trait `schedule::Pass` is in another module and is not glob re-exported, so it does not collide).

### S1 scan (K.1): `Renewal` and `Start`

```rust
/// When the walk's lease was last renewed, and how long it may go without one (`Pacing::renew_every`).
struct Renewal {
    at: Instant,
    every: Duration,
}

impl Renewal {
    fn new(every: Duration) -> Renewal {
        Renewal { at: Instant::now(), every }
    }
    /// The lease has gone `every` without a renewal.
    fn due(&self) -> bool {
        self.at.elapsed() >= self.every
    }
    /// The lease was just renewed (a batch's write renews it too).
    fn renewed(&mut self) {
        self.at = Instant::now();
    }
}

/// Where a walk starts: the root folder's path, its open handle, its device and its volume.
struct Start<'a, D> {
    path: &'a RulePath,
    dir: D,
    device: Option<u64>,
    volume: u64,
}
```

New signatures (parameter counts; the lint fires above 7):

```rust
fn take(pool: &Pool, key: &Key, ctl: &Control, lease: &ScanLease<'_>, renewal: &mut Renewal) -> Result<Taken>   // 5 (was 6)
fn list_here<L: Listing>(lease: &ScanLease<'_>, renewal: &mut Renewal, lister: &mut L, rules: PathRules,
                         pool: &Pool, job: &Job, dir: Option<L::Dir>) -> Result<Outcome>                       // 7 (was 8)
fn checkpoint(lease: &mut ScanLease<'_>, root: &RulePath, ctl: &Control, batch: &mut Batch,
              stats: &mut ScanStats, renewal: &mut Renewal, pool: &Pool) -> Result<bool>                       // 7 (was 8)
fn walk_leased<L: Listing>(lease: &mut ScanLease<'_>, start: Start<'_, L::Dir>, ctl: &Control,
                           lister: &mut L, pacing: Pacing) -> Result<ScanStats>                                 // 5 (was 8)
```

Edit list (line numbers from the snapshot read; find by text):

- `take` (`:686-723`): drop `renewed: &mut Instant` and `renew_every: Duration`; `if checked.elapsed() >= renew_every`
  becomes `>= renewal.every`; `*renewed = Instant::now();` becomes `renewal.renewed();`.
- `list_here` (`:793-824`): the `beat` closure becomes
  `if lost.is_none() && renewal.due() { match renew(lease) { Ok(()) => renewal.renewed(), Err(e) => lost = Some(e) } }`
  (it captures `renewal` mutably and `lost`, as it captured `renewed` before; the `rows` closure still borrows `lease`
  shared). Delete the `#[expect(clippy::too_many_arguments, ...)]` at `:792`.
- `checkpoint` (`:1269-1303`): `if renewed.elapsed() >= renew_every { renew(lease)?; *renewed = Instant::now(); }`
  becomes `if renewal.due() { renew(lease)?; renewal.renewed(); }`; the two other `*renewed = Instant::now();` (`:1272`,
  `:1285`) become `renewal.renewed();`. Delete the expect at `:1269`.
- `walk_leased` (`:973-983`, delete the expect at `:972`): first line of the body
  `let Start { path: root, dir: root_dir, device: root_device, volume: root_volume } = start;` (so every later use of
  `root`, `root_dir`, ... is unchanged); inside the scope closure `let mut renewed = Instant::now();` (`:1004`) becomes
  `let mut renewal = Renewal::new(pacing.renew_every);`; the calls at `:1036`, `:1046`, `:1052`, `:1061` pass
  `&mut renewal` and drop `pacing.renew_every`; `renewed = Instant::now();` after `write_batch` (`:1212`) becomes
  `renewal.renewed();`.
- `walk_paced` (the one caller, `:897-898`; `walk` at `:848` only calls `walk_paced`):
  `walk_leased(&mut lease, Start { path: root, dir: root_dir, device: root_device, volume: root_volume }, ctl, lister,
  pacing)`.
Borrow check: `Renewal` is a plain value owned by the scope closure and lent `&mut` to one callee at a time, as `renewed`
was; `Start` is moved into `walk_leased` and destructured at once, so `root` stays a `&RulePath` and nothing else in the
body changes. This corrects the earlier note that `root` would be used as `.path` in `checkpoint` and `write_batch`: it
is not, because of the destructuring.

### S2 jobs (K.1): `Pass`

```rust
/// What a per-file pass runs over and under: its items, its options and its control. [`run`] and [`batched`] take one.
pub struct Pass<'a, T> {
    /// What the pass runs over: item `i` is read, processed and sunk by its index.
    pub items: &'a [T],
    /// The pass's readers, workers, buffers and priority.
    pub opts: &'a Options,
    /// The pass's pause, stop and progress.
    pub ctl: &'a Control,
}
```

No `derive(Clone, Copy)`: the derive would add `T: Clone`/`T: Copy` bounds, and nothing needs `Copy` (each function
destructures `pass` once, binding only `&` fields, and `batched` passes `pass` whole to `run_timed` as its last use of it).
If a caller wants to reuse one `Pass`, add `impl<T> Clone for Pass<'_, T> { fn clone(&self) -> Self { *self } }` and
`impl<T> Copy for Pass<'_, T> {}` by hand.

```rust
pub fn run<T, S, R, I, P>(pass: Pass<'_, T>, read: impl Fn(&T, &mut NativePath) -> Read + Sync,
    init: impl Fn() -> Result<S, I> + Sync, process: impl Fn(&mut S, usize, &[u8]) -> Result<R, P> + Sync,
    sink: impl FnMut(usize, Result<R, Failed>, usize) -> Result<bool, Failed>) -> Result<(), Failed>      // 5 (was 7)
fn run_timed<T, S, R, I, P>(pass: Pass<'_, T>, read: .., init: .., process: .., mut sink: .., timing: &Timing)
    -> Result<(), Failed>                                                                                  // 6 (was 8)
pub fn batched<T, S, R, W, I, P>(pass: Pass<'_, T>, read: .., init: .., process: .., batch: Batch, writer: W,
    flush: impl FnMut(&mut W, &mut Vec<(usize, Result<R, Failed>)>) -> Result<bool, Failed> + Send)
    -> (W, Result<(), Failed>)                                                                             // 7 (was 9)
```

Edit list:

- Each of the three starts with `let Pass { items, opts, ctl } = pass;`, so their bodies are unchanged. `batched`'s call
  `run_timed(items, read, opts, ctl, init, process, <sink closure>, timing)` (`:490-514`) becomes
  `run_timed(pass, read, init, process, <sink closure>, timing)`. `run`'s call at `:226` likewise
  (`run_timed(pass, read, init, process, sink, &timing)`).
- Delete the `#[expect(clippy::too_many_arguments, ..)]` on `run_timed` (`:232`) and on `batched` (`:434-438`): at 6 and 7
  parameters neither lint fires, so each would be an unfulfilled expectation.
- Every caller: `run(&items, read, &opts, &ctl, init, process, sink)` becomes
  `run(Pass { items: &items, opts: &opts, ctl: &ctl }, read, init, process, sink)`; the same for `batched`. Callers:
  `batched` in `borders/lib:169`, `import/hashes:54`, `import/lib:132`, `thumbs/lib:407`, `thumbs/placeholders:95`; `run` in
  `cli/borders:80`, `crop/lib:171,325`; 13 test callers in `pipeline.rs` (7 `run`, `:1031-1341`; 6 `batched`,
  `:1512-1642`). `run_timed` has no other caller than `run` and `batched`. (Counts verified; line numbers drift, grep.)
`T: Sync` stays on the functions (not on `Pass`).

### S3 codec (K.1): `Request`

In `crates/codec/src/decode.rs` (it imports `crate::error::{Error, Result}`; Phase 7 moved `Error` into `error.rs`, so the
two `Error` variants of D.2 go there):

```rust
/// What a decode is asked for: the file's format (`Format::sniff`), its EXIF orientation (`pg_metadata::orientation`, 1
/// without one), the least size ([`MinSize`]) and the catalog's quarter turns.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// The file's format (`Format::sniff`).
    pub format: Format,
    /// The file's EXIF orientation (1 without one).
    pub exif: Orientation,
    /// The least size the pixels may be decoded to.
    pub min: MinSize,
    /// The catalog's quarter turns, composed with `exif`.
    pub turns: Turns,
}

pub fn decode_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb, spare: &mut Vec<u8>) -> Result<Info>   // 5 (was 8)
pub fn decode_stored_into(&mut self, bytes: &[u8], request: Request, out: &mut Rgb) -> Result<Info>                  // 4 (was 7)
```

Edit list: `decode_into` calls `self.decode_stored_into(bytes, request, out)?`; `decode_stored_into` starts with
`let Request { format, exif, min, turns } = request;` and its body is unchanged; delete the expect at `:130`; `dimensions`
(5 with `self`, no lint) keeps its arguments. The callers (grep over all `*.rs` finds exactly these, no fuzz, bench or eval
Rust file): `analysis/src/analyze.rs:122,138`, `analysis/examples/stage_times.rs:44`, `render/src/display.rs:40,54`,
`edit/src/lib.rs:90` (inside `testing::stored`, `decode_stored_into`), the codec tests `decode.rs:376,429,503`, and the
internal call `decode.rs:133`. Each builds `Request { format, exif, min, turns }` from the four arguments it passes today
(`turns` is the caller's value, or `Turns::NONE`, which exists, where it passes none). `maker.rs:179,190,202` and
`display.rs:60` go through `render::display::decode_into`, a wrapper that computes `format` and `exif` itself
(`display.rs:37-40`) and keeps its own signature, so they do not change. `analysis`, `render` and `edit` each add
`Request` to their `use pg_codec::decode::{...}`; the codec tests get it through `super::*`. `out` and `spare` stay
separate `&mut` parameters. `Request`'s doc names `pg_metadata::orientation` as plain text, not an intra-doc link
(`pg-codec` does not depend on `pg-metadata`).

### S4 `jpeg.rs` in codec, and `crop.rs` (D.2, D.3)

The moved `rewrite/jpeg.rs` after the error mapping (`Error::decoder` and `Error::encoder` exist in
`codec/src/error.rs:42,50` and set `step: error.to_string()`, so the text each `Error::bare` showed is unchanged; they
need `turbojpeg::Error: std::error::Error + Send + Sync + 'static`, which compiling will confirm):

```rust
use crate::{Error, Result};
use turbojpeg::{Transform, TransformCrop, TransformOp, Transformer};
use super::op::Op;

pub fn rewrite(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    /* body as today, except: */
    let Some(progressive) = pg_container::jpeg_is_progressive(bytes) else {
        return Err(Error::Imperfect("the JPEG's structure is damaged before its scan".into()));
    };
    /* ... */
    match Transformer::new().map_err(Error::decoder)?.transform_to_vec(&t, bytes) {
        Ok(out) => Ok(out),
        Err(e) if perfect => Err(Error::Imperfect(e.to_string())),
        Err(e) => Err(Error::encoder(e)),
    }
}

/// The JPEG's minimum coded unit, width and height: the grid a lossless crop starts on.
pub fn mcu(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes).map_err(Error::decoder)?;
    Ok((header.subsamp.mcu_width(), header.subsamp.mcu_height()))
}

/// The JPEG's width and height in pixels, from its header.
pub fn size(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes).map_err(Error::decoder)?;
    Ok((header.width, header.height))
}
```

`crates/edit/src/crop.rs:43-55` after the change:

```rust
let (mw, mh) = rewrite::jpeg::mcu(bytes)?;
let grid = |n: u32, step: usize| u32::try_from(step).map(|s| n.next_multiple_of(s)).map_err(|e| Error::invalid(e.to_string()));
let (l, t) = (grid(l, mw)?, grid(t, mh)?);
/* ... unchanged ... */
let out = rewrite::jpeg::rewrite(bytes, exact(l, t))?;
let got = rewrite::jpeg::size(&out)?;
let want = (num::index(width - l - r), num::index(height - t - b));
if got != want {
    return Err(Error::invalid(format!("the crop came out {}×{}", got.0, got.1)));
}
```

(`?` converts `pg_codec::Error` through `Error::Codec(#[from])`.) The `Imperfect` text is identical to today's; edit's
`rotate.rs:61,74` then match `Err(pg_codec::Error::Imperfect(_))` as in D.2.

### S5 Phase H: the `pg-model` modules and the replacement functions

New files in `crates/model/src/` (each a `//!` doc line plus the items moved **verbatim**, doc comments and derives
included, with `use` lines pointing at the same items under `crate::`); add `pub mod cache; pub mod facets; pub mod inbox;
pub mod job; pub mod ledger; pub mod removal;` to `lib.rs` (keep its alphabetical order) and extend its module list doc
(the outcomes the front ends word: inbox entries and counts, the folder ledger, facet counts, removals, job events and
causes, the thumbnail cache's cap). Item by item, with the source to move from:

| New file               | Items moved verbatim                                                                                            | From                                                                    |
|------------------------|-----------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| `cache.rs`             | `ThumbCap` (was `Cap`), `const MAX_CAP_BYTES`, its `impl`, the test `a_cap_is_a_positive_number_of_gib_as_text` | `thumbcache/src/config.rs:33-64, 77-91`                                 |
| `facets.rs`            | `Facets`                                                                                                        | `catalog/src/browse/read/facets.rs:20-32`                               |
| `inbox.rs`             | `InboxEntry`, `InboxEntryId` + `fn entry`, `InboxCounts`                                                        | `catalog/src/inbox.rs:72-`, `inbox/id.rs:5-27`, `ledger/state.rs:61-66` |
| `ledger.rs`            | `FolderFailure` + `key`/`from_key`, `FailedFolder`, `FailedFile`                                                | `catalog/src/types.rs:151-177`, `ledger/state.rs:68-`, `ledger.rs:140-` |
| `removal.rs`           | `Begun`                                                                                                         | `catalog/src/removal.rs:36-`                                            |
| `job.rs`               | `Change`, `Cause` + its `Display`                                                                               | `jobs/src/events.rs:128-143`, `jobs/src/failure.rs:163-191`             |
| `refusal.rs` (exists)  | `CatalogRefusal` (was `Refusal`, no `message`)                                                                  | `catalog/src/error.rs:97-125`                                           |
| `write.rs` (exists)    | `WriteError` (no `Display`)                                                                                     | `writeback/src/lib.rs:90-98`                                            |
| `progress.rs` (exists) | `LaneProgress`                                                                                                  | `library/src/shared.rs:125-`                                            |

"Verbatim" has one exception: rustdoc links to items that stay behind become plain code spans, because `pg-model` cannot
link up the layers (`Entry`'s `[`crate::ledger::Summary::seen_to`]`, `Begun`'s `[`Catalog::begin_removal`]`,
`Refusal::OlderSchema`'s `[`crate::Catalog::open`]`; grep each moved item for the bracketed-link form). `Cap`'s second doc sentence (the
`--cap-gib` and `PG_THUMB_CAP_GIB` text) stays in the model's docs, as it describes the value's users; and the
`cache.rs` snippet below keeps today's per-method docs (an earlier draft dropped them, which `missing_docs` rejects). Write
`write.rs` imports `crate::job::Cause`, and `ledger.rs` imports `crate::ids`, `crate::notice::LedgerKind` and
`crate::rules::Path` for its fields (checked: every field type is already in `pg-model`).

`cache.rs` as it should read (only the names and the `num` path change):

```rust
//! The thumbnail cache's size limit as a value.

/// A cache's size limit, in bytes: above zero and under `i64::MAX`, so no cap evicts every thumbnail or saturates. The
/// front ends read the number (the command line's `--cap-gib`, the app's `PG_THUMB_CAP_GIB`) and word their own errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbCap(i64);

/// The largest cap, in bytes: under `i64::MAX` (about 9.22e18).
const MAX_CAP_BYTES: f64 = 9.0e18;

impl ThumbCap {
    /// `gib` GiB (2^30 bytes) as a cap, the fraction of a byte dropped; `None` for zero, negative, NaN, infinite and
    /// absurd sizes.
    pub fn from_gib(gib: f64) -> Option<ThumbCap> {
        let bytes = gib * 1_073_741_824.0;
        (1.0..MAX_CAP_BYTES).contains(&bytes).then(|| ThumbCap(crate::num::f64_to_i64(bytes)))
    }

    /// The cap a number of GiB written as text names: the `--cap-gib` flag and the `PG_THUMB_CAP_GIB` variable both read
    /// it this way (see [`ThumbCap::from_gib`]). Surrounding spaces are ignored.
    pub fn parse_gib(text: &str) -> Option<ThumbCap> {
        text.trim().parse::<f64>().ok().and_then(ThumbCap::from_gib)
    }

    /// `bytes` as a cap; `None` unless it is above zero.
    pub const fn from_bytes(bytes: i64) -> Option<ThumbCap> {
        if bytes > 0 { Some(ThumbCap(bytes)) } else { None }
    }

    /// The cap in bytes, as the cache stores it.
    pub const fn bytes(self) -> i64 {
        self.0
    }
}
```

(The method docs are today's, moved unchanged: they now satisfy `missing_docs` as well as describing the front ends.)

The functions and impls that stay in the owning crates:

```rust
// catalog/src/error.rs: the attribute and the private function (the arms are today's `Refusal::message`, unchanged)
#[error("{}", refusal_message(.reason, .path))]
Refused { path: String, reason: Box<CatalogRefusal> },

/// The text of this refusal for the catalog at `path`.
fn refusal_message(refusal: &CatalogRefusal, path: &str) -> String {
    match refusal {
        CatalogRefusal::NewerSchema { found, current, written_by } => { /* as today */ }
        /* ... the other eight arms, as today ... */
    }
}

// jobs/src/failure.rs: replaces `impl Cause { pub fn of(..) }`
impl From<&Failed> for Cause {
    fn from(failed: &Failed) -> Cause {
        if failed.is_panic() { Cause::Panicked(failed.text().to_string()) } else { Cause::Error(failed.text().to_string()) }
    }
}
// call sites: failure.rs:149 (Cause::from(failed)), the tests at :211 and :219 (Cause::from(&failed)), writeback/src/lib.rs:489 (Cause::from(&e))

// writeback/src/lib.rs: replaces `impl Display for WriteError`
/// The English words for a write error, for the command line and the queue's `error` column; the app words it through
/// `pg_wording`.
pub fn write_error_words(error: &WriteError) -> String {
    match error {
        WriteError::Failed(cause) => cause.to_string(),
        WriteError::CopyIncomplete { file, temp, cause } => format!(
            "{file} may be incomplete: copying its change into it failed ({cause}); the whole new content is in {temp}"
        ),
    }
}
// call sites: writeback/src/lib.rs:499 (`&error.to_string()` -> `&write_error_words(&error)`), cli/src/lib.rs:720, the test near :887
```

`Facets` stays built by `impl Catalog { pub fn facets(..) -> Result<Facets> }` (an inherent impl on the local `Catalog`,
legal after the move). The catalog-side imports become `use pg_model::{facets::Facets, inbox::{InboxCounts, InboxEntry,
InboxEntryId}, ledger::{FailedFile, FailedFolder, FolderFailure}, refusal::CatalogRefusal, removal::Begun}`; the SQL call
sites of `FolderFailure::key`/`from_key` (`ledger.rs:307`, `roots.rs:246`, `writers.rs:203`) are unchanged.

---

## Allowlist content that stays, with the reason

- `tools/crate-layers.txt`: it is the rule (exact internal edges, reviewed in the same commit as the change).
- `deny.toml` `licenses.allow`: a policy list, pruned to what is used (Phase B step 6).
- `ENV_READERS`: the registry reader's two functions (`process`, `set_process`), nothing else.
- bindgen `TYPES`/`VARS`: the mechanism that bounds generated FFI; kept, with Phase F's check.
- Single-owner bans (`ureq`, `quick-xml`, `cxx-qt`, `cxx-qt-lib`, `gif`, `webp`, `tiff`, `zune-*`): real owners.
- `deny.toml` `allow-git`: a source policy naming the pinned forks that stay (Phase G).
- The tooling and CI exceptions that stay: the `ruff.toml` global suppressions `S603`, `S607` and `TRY003` (used in
  `tools/`), the per-file ignores (`eval/**` and the tools' imports inside functions), the four-name `TEST_INPUTS` list in
  `tools/test_ci_files.py` (the other test inputs are constants in `crates/testkit/src/inputs.rs`), and
  `modgraph/split_check.py` `SKIP`.
- `UNSAFE_CRATES` (`tools/test_ci_files.py`): the CU-005 rule, self-checking through its guard tests.
- Phase K keeps: the `unsafe_code` expects (66, 67 once Phase C adds `env.rs`'s; minus the K.10 sites that go; FFI and OS
  calls), `as_conversions` and the casts in `num.rs`, the owners of banned calls, the log fallback's printing,
  `platform/src/process.rs`'s `exit`, fault-injection panics, the generated-code expects if K.7's single module does not
  build, and the three `indexing_slicing` sites `pdq.rs:78`, `join.rs:1017` and `join.rs:1070`. Also the `unwrap_used`
  expect of `catalog/src/fixture/raw.rs` (outside test builds only, under the `fixture` feature: its 38 helpers are the
  statements the catalog's fixture makes, and a failing statement is the caller's failure, not a result to return).

## Decisions (settled: the most proper option, churn not a concern)

1. Phase E: the DLL check **fails** the step. Unknown DLLs are either added to `REDISTRIBUTED` with a reason or are a real
   problem; any legitimate OS DLL the first real build flags is added deliberately.
2. Phase G: forks stay forks (the repo's recorded decision); nothing is vendored in-repo. `parajpeg` keeps its pinned fork,
   the mutex fix included.
3. Phase G: each fork change is measured, then kept in its pinned fork with the gain written in `vendor/README.md`, or
   dropped. No upstream pull requests: the user declined them.
4. Phase H runs before Phase I; the app entry leaves `REEXPORT_BOUNDARIES` for good.

## Rejected approaches (and why), so they are not re-proposed

- Vendoring `parajpeg` in-repo to empty `allow-git`: contradicts the repo's recorded move from in-repo copies to forks; its
  upstream C++ is unchanged and our commits sit on top, which is what a fork is for.
- Mirror types in `pg-app` for the 14 types: they are plain data, so mirrors only duplicate them.
- A std-only leaf crate below `platform` for the test folders: `platform/src/testing.rs` reads `crate::env` and
  `windows_sys`, and platform's own tests need the helpers.
- A `just bench` recipe as the measurement instrument: it does not exist.
- A System32-existence check for the DLL policy: it answers for the build machine's Windows, not the supported targets.
- A new `pg_codec::rewrite::Error` enum: `pg_codec::Error` already has the variants and helpers; two variants are added.
- Moving `Refusal::message` or `WriteError`'s `Display` into `pg-model`: `pg-model` says "Nothing here is English".
- Keeping `Cap`'s constructors in `thumbcache` while the type moves: the whole inherent `impl` must move (orphan rule).
- Moving `CACHE_KIB`/`WRITE` into `pg-catalog` so the probes can move: they are pairing-pass tuning values, not catalog data.
- Probes taking the values as positional arguments or with defaults: positionals collide with variant names; mirrored
  defaults drift.
- `std::fs::copy` alone in the bench: the copied files are non-WAL and `pending_thumbs` needs WAL.
- A `Stages` trait for `jobs::run_timed`/`batched`: `read`/`init`/`process` are `Fn + Sync` shared across threads while
  `sink`/`flush` are `FnMut` (and `flush` is `Send`), so one implementor would need `&self` shared and `&mut self` at once,
  and every caller would become a hand-written struct.
- A single `Walker<'a, L>` for `scan`: a field `lease: &'a mut ScanLease<'a>` pins the lease's own lifetime to the outer
  borrow (`&mut T` is invariant in `T`; `ScanLease<'c>` itself is covariant in `'c`, as it holds `&'c mut Catalog` and
  `Catalog` has no lifetime), so the lease is borrowed for `'a` for good and the loop's later direct uses fail; it needs
  two lifetimes and rewrites the roughly 300-line loop body.
- A buffers struct for `decode_into`'s `out` and `spare`: built per call from two disjoint fields, it gains nothing.
- Registering test inputs as `VARS` rows: `VARS` is compiled into production; `env.rs:93` says tests' inputs are not there.
- Making `BASELINE_VERSION` private to leave `NOT_REGISTERED`: the scan regex ignores `pub`.
- Reading build-script variables through `pg_platform::env`: `env::process` reads an empty value as unset, while a build
  script reads cargo's own variables where presence is the signal (`desktop/build.rs:121` is a presence-only check on
  `CARGO_CFG_DEBUG_ASSERTIONS`).
- A Hamming-distance tolerance for the PDQ vectors if Linux disagrees: a platform-dependent hash is a correctness bug.
- Raising `multiple-versions` to `deny` in Phase B: the known duplicates unrelated to `pdqhash` would fail it until each has
  a `[[bans.skip]]` entry and a reason. That is the right end state (Phase B step 6 says so), but it is a separate
  decision after Phase G, not part of removing `pdqhash`.
- `#[doc(hidden)] pub` and `pub(crate)` for the desktop's bridge state structs (K.7): the first creates API, the second
  is E0446.
- `eval/jpeg-decoders` as the instrument for the MSVC `turbojpeg-sys` change: it is the Linux/container harness and its
  Windows script builds libjpeg-turbo with `/O2`.
- `tools/fork-updates.py` as a stale-pin detector: it only compares versions with crates.io.
- Treating S101/S311/PLC0415 as eval-only `ruff` ignores: they occur in `tools/`.
