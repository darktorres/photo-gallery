# Tracking values end to end: ideas for later

Status: ideas only, not planned work. Nothing here is implemented, and no tool named below is installed in this repo's toolchain. The facts below were re-checked against the working tree, a probe crate (outside the repo) and the tools' upstream repos on 2026-11-25. Line numbers drift; function names are the stable reference.

## The question

Can we track a value from where it enters the program (a file's bytes, a QML property, a remote response) to where it is used (a SQL bind, a slice, a QML property, a network fetch), the way Psalm's taint analysis tracks untrusted input to its sinks in PHP?

## Found while checking: fix before any provenance work

These are real defects or gaps, independent of the wrapper.

- **Yandex result URLs are fetched from any host.** `Fetcher::download` (`crates/yandex/src/fetch.rs`) GETs whatever URL the Yandex page returned (`OtherSize.url`, or `SimilarItem.image_url` decoded from `img_url`). No host or IP check, so loopback, LAN and link-local addresses are reachable, and nothing in the crate sets a redirect policy. Downloads start as soon as results arrive, four at a time (`crates/app/src/yandex/images.rs`), up to 100 MB each, and the bytes go straight into the decoders (`grid_picture`). Fix: a host/IP policy (refuse private, loopback and link-local after DNS resolution) and an explicit redirect cap. `file://` is already impossible (ureq is http(s) only), and the saved file name uses only numbers and the sniffed extension.
- **The FFmpeg context sets no `protocol_whitelist`.** The pinned static build is `--disable-network --enable-protocol=file` with five demuxers (`tools/build_static_ffmpeg.py`), so release builds are safe. A dev build against a full FFmpeg (`FFMPEG_DIR`) is not: a content-probed HLS or concat demuxer could open nested URLs. Set `protocol_whitelist=file` on the context in `crates/video/src/ffi.rs`, so the safety does not depend on which FFmpeg is linked.
- **An old catalog is attached with `trusted_schema` on.** `rebuild.rs` attaches the old file and runs `SELECT … FROM carried.{table}`; `pragma_table_info` also matches views, so a crafted old file's view runs. Only built-in and registered functions are reachable. Fix: `PRAGMA trusted_schema = OFF` (or `SQLITE_DBCONFIG_DEFENSIVE`) on connections that attach or inspect files. The thumbnail cache's `ATTACH` is the same shape.
- **`file:` catalog paths are URIs.** The SQLite fork builds with `-DSQLITE_USE_URI`, so a relative path beginning with `file:` would be parsed as a URI. Absolute paths cannot hit it.
- **Small overflows:**
  - `s.pos + 1` (`crates/app/src/yandex.rs`, the "yandex similar" suffix) overflows when `pos` is `u32::MAX`, which `parse.rs` accepts.
  - `preview_item + by` in `step_preview` (`crates/desktop/src/yandex.rs`) is unchecked `i32` math on a QML argument.
  - `png_grid_search` (`crates/container/src/lib.rs`): `step = len.saturating_add(12)` can saturate to `usize::MAX`, and `run_at + (lo + 1) * step` then overflows. 32-bit targets only.
  - GIF frame counting (`images += 1` in `crates/metadata/src/container.rs`) is unchecked, unlike WebP's `saturating_add`. Unreachable in practice (over 4.29 billion image blocks).
- **The Yandex status line.** The URL reaches the status text through error wording (`crates/wording`), shown in `YandexPanel.qml`. That `TextEdit` is the only dynamic text without an explicit `textFormat: TextEdit.PlainText` (53 others set it). Qt 6's default is plain text, but set it explicitly.

## What Rust has

Rust has no built-in taint analysis. These are the closest options, in the order to try them.

### 1. Types, enforced by rustc (the most proper option here)

Wrap values that come from outside the program in a type with private fields and no `Deref`, `Into` or `From`. Its only way out is a validator that returns a domain type.

What the probe confirmed (repo toolchain `nightly-2026-11-06`, repo lints, a second crate standing in for `pg-metadata`): a downstream `.into()` (E0277), field access (E0616), `Display` (E0277), `*` (E0614), and `==`/`<` (E0369) on the wrapper are all rejected by rustc.

**The validator must not be a closure.** `fn checked<D, E>(self, f: impl FnOnce(T) -> Result<D, E>)` hands the raw value to any closure, and the probe got it back three ways: `checked(|v| Ok(v))`, `checked(|v| Err(v)).err()`, and a closure that writes `v` into a captured variable and returns `None`. No bound on `D` or `E` stops the third.

**A sealed trait does hold.** In the probe, no downstream route got a raw `u32` out:

```rust
#[must_use = "an untrusted value is checked or dropped"]
#[derive(Clone, Copy)]
pub struct Untrusted<T>(T);

impl<T> Untrusted<T> {
    pub const fn new(raw: T) -> Self { Self(raw) }
    pub fn checked<D: FromUntrusted<T>>(self) -> Result<D, D::Error> {
        D::from_raw(self.0, private::Token)
    }
}

impl<T> fmt::Debug for Untrusted<T> { /* writes "Untrusted(<unchecked>)" */ }

mod private { pub trait Sealed {} pub struct Token; }

pub trait FromUntrusted<T>: Sized + private::Sealed {
    type Error;
    #[doc(hidden)]
    fn from_raw(raw: T, token: private::Token) -> Result<Self, Self::Error>;
}
```

A downstream `impl FromUntrusted` fails (E0277, `Sealed` not satisfied), and naming `private::Token` fails (E0603). `from_raw(x, todo!())` compiles, but it only panics, and it only turns a raw value into a domain value, so nothing comes out.

What it does not give:

- **Domain constructors take raw integers.** `Rating::new(u8)` and `Rating`'s `TryFrom<u8>` are public, so a raw `u8` can become a `Rating` without touching the wrapper. They range-check, so this matters less than it sounds (open question 6).
- **A domain type with a public payload re-opens the seal.** `Ok(Taken::Wall(s))` gives back a plain `i64`. Validated, but no longer typed. `Taken` should get a private payload (open question 6).
- **pg-model can implement the trait for anything.** `impl FromUntrusted<u32> for u64` would be a full exit, and only review stops it. Rule: implement it only for pg-model newtypes with private payloads, never for std types.
- **A context validator must return the checked thing, not an index.** An offset checked against a slice should come out as the sub-slice (`fn slice<'a>(self, bytes: &'a [u8], len: usize) -> Option<&'a [u8]>`), or the returned `usize` is a raw integer again.
- **Comparison and hashing are exits.** A test wrapper with a private field and derived `PartialOrd` leaked its exact value by binary search, and with derived `Hash` through a custom `Hasher`. Never derive `PartialEq`, `PartialOrd`, `Hash` or `Default` on the wrapper.
- **`transmute`** is stopped only by `unsafe_code = "deny"`. Do not add `#[repr(transparent)]`, which would make the transmute well-defined.
- **A validator is an oracle.** `u.checked::<Frames>().is_ok()` leaks one bit per validator. Inherent and harmless for the threat in question 5.

`Copy` is safe: a copy is still sealed. `#[must_use]` fires on a bare `Untrusted::new(5);`.

So rustc checks the wrapper's seal. It does not check that every path to a sink passes a validator. That needs a dataflow tool (option 3).

### 2. Lints that enforce the boundary

The repo already denies `as_conversions`, `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `cast_lossless`, `cast_precision_loss`, `indexing_slicing`, `unwrap_used`, `expect_used` and `panic` (`Cargo.toml`, in `[workspace.lints.clippy]`). `arithmetic_side_effects` is not enabled. All 11 production `as` casts sit in `crates/model/src/num.rs`, each under `#[expect]`, so no silent `as` cast sits between the layers.

`clippy.toml` already uses `disallowed-types` (for `HashMap`) and `disallowed-methods` (for `temp_dir` and two `pg_platform::names` paths). The same mechanism can ban an escape hatch. The probe checked that clippy flags an `into_inner` in three forms: a method call, a UFCS path call, and passing it as a fn value. Limits:

- **It is crate-wide.** A call from inside the wrapper's own module is flagged too. It does not flag a private field read (`self.0`).
- **It does not see fields.** A `pub(crate)` field read is not flagged. The fields stay private, enforced by rustc.
- **An entry for a method that does not exist warns on every run** (`does not refer to a reachable function`). It is a config warning, so `-D warnings` does not make it an error, but it is noise. With the sealed-trait design there is no `into_inner`, so the seal is kept by the types, and the entry is added only if an accessor is ever added. If one is unavoidable, name it `*_less_safe`, as the `untrusted` crate does, and ban it.

Negative tests belong with the wrapper. `compile_fail` doctests run in this repo (`just test` is `cargo test --workspace`; only `crates/desktop` sets `doctest = false`), and they would be pg-model's first doctests. Each must name its error code (`compile_fail,E0616`): this nightly enforces the code, while a bare `compile_fail` passes on a typo. The code must be present, not the only error, so keep each snippet minimal. One test each for `.into()` (E0277), field access (E0616), `Display` (E0277), deref (E0614), comparison (E0369) and a downstream `impl FromUntrusted` (E0277). trybuild is not worth adding: its `.stderr` files change with nightly wording on every toolchain bump. Neither tool can test a clippy entry.

A custom lint is possible through **Dylint** (v6.4.0, 2026-11-10, Windows in CI): for example "a catalog writer's parameters must be pg-model domain types, never raw `u32`/`i64`". Only if clippy's `disallowed-*` proves too coarse.

### 3. Whole-program dataflow and proofs

- **CodeQL for Rust** is the closest thing Rust has to Psalm. It is generally available since 2025-10, and the CLI (v2.27.6, 2026-11-24) ships a Windows build. It does source-to-sink taint tracking with sanitizers (barriers), and sources and sinks can be declared as `.model.yml` rows. It already models `rusqlite`'s `execute`/`prepare` as SQL sinks and `std::fs::read` as a `file` source (that source kind is off by default). It extracts through rust-analyzer without building, so the cxx-qt build does not block it. Here: the readers as sources, the catalog writers, bridge setters and `Fetcher::download` as sinks, and the `FromUntrusted` validators as barriers. Its built-in queries look for injection; the "semantically wrong value" threat (question 5) needs a custom query. Macro-generated bridge code and FFI are seen only as far as rust-analyzer expands them. This is the tool that checks what rustc cannot: that every path to a sink passes a validator.
- **MIRAI** is archived at `facebookexperimental/MIRAI` (last push 2024-08-22). A fork continues at `endorlabs/MIRAI`, last commit 2025-03-04, pinned to `nightly-2025-01-10`: too old for this repo's toolchain. Do not build on it.
- **Flowistry** is active (last push 2026-05-22, v0.5.44, `nightly-2026-05-01`, Windows builds). It answers "what influences this value?" within one function, and its IDE focus mode shows the code related to a chosen variable. Its README says it does not analyse the bodies of called functions (it guesses their effect from type signatures), cannot analyse closures or async together with their callers, and has a gap with interior mutability. The small `flowistry_ifc` crate with `Secure`/`Insecure` markers is a one-function research demo. Not a whole-program tool.
- **Paralegal** (Brown, built on Flowistry, OSDI 2025, v0.1.0 2026-06-15) is the policy-based source-and-sink checker Flowistry is not. No Windows build, it can miss flows through `unsafe`, `RefCell` and `Mutex`, and markers cannot go on fields. Skip.
- **Kani** (v0.68.0, 2026-09-16) checks overflow, failed `try_from`, panics and out-of-bounds indexing with `kani::any()`, no specifications needed. The experimental `cargo kani autoharness` writes harnesses itself. **It does not run on Windows** (issue #1954, open since 2022): only in the Linux container, as `cargo kani -p pg-model`, which keeps the cxx-qt crates out.
- **Prusti** (now `prusti/prusti`, daily nightly prereleases, last full release 2024-02-28) proves absence of overflow and panics **without specifications** by default. It has a Windows build and needs Java. A 2026 study (arXiv 2610.21373) found it crashed or hung on 92.5% of real-world cases, so expect it to reject common std and trait usage.
- **Creusot** (v0.15.0) needs specifications and has no Windows CI. **Verus** means rewriting code in `verus!` syntax. **Flux** (refinement types such as `u32{v: v > 0}`) runs on Windows but is research-grade. **Charon/Aeneas** and **hax** translate to proof languages. None fits.
- **Not for this question:** RAPx (memory safety), Rudra and SafeDrop (archived), MirChecker (stale), cargo-geiger (counts `unsafe` only).

### Prior art for the wrapper

- **`untrusted`** (briansmith, used by ring and webpki) wraps bytes, parsed through `read_all` and `Reader`. Its one raw exit is named `as_slice_less_safe()`. Its hand-written `Debug` prints only `Input`, and it deliberately has no `PartialEq`. No serde.
- **`secrecy`** exits through `ExposeSecret`, prints `[REDACTED]`, makes `Clone` and `Serialize` opt-in, and allows `Deserialize`. That last choice is the opposite of this plan's, and this plan's is right here.
- **`untrusted_value`** (0.3.2) is closest to this design (`sanitize_with`, no `Debug`/`Display`), but it has a public `From<T>` and a raw `use_untrusted_value()`, and its crate-wide taint check is future work.

## Limits of all of these

- `unsafe` code and FFI are not seen. 267 source lines under `crates/` open an `unsafe` block, function, impl or extern (272 contain the word at all): video 106, platform 79, desktop 71, dedup 8, and 3 in `catalog/examples`, none in catalog production code. The grep cannot see the `unsafe` cxx-qt generates. `crates/metadata` has `#![forbid(unsafe_code)]`.
- QML is untyped JavaScript. A value's journey stops at the Rust receiver.
- A `serde` impl can build a domain type straight from input. That is safe only when the impl goes through the validator. `Rating` does (`#[serde(try_from = "u8")]`).
- A derived `Serialize` or `Deserialize` on a struct that holds the wrapper fails to compile (E0277), so the wrapper cannot leak through serde unless someone implements it by hand. A derived `Debug` on such a struct uses the wrapper's own `Debug` and prints `Untrusted(<unchecked>)`.

## What the repo carries today

The file-derived values that reach the database raw are the gap:

- `Meta.orientation` is an `Orientation(u8)` newtype with a private field, and `Meta.rating` is `Option<Rating>`. Both are typed.
- `Meta.taken` is `Option<i64>`, wrapped as `Taken::Wall(i64)` in `crates/analysis/src/analyze.rs`. `Taken` is a pub enum with pub variants and no validating constructor: any `i64` becomes one. Video drops dates at or before 1970; EXIF does not, so a pre-1970 EXIF date goes in negative. It reaches `store_analysis` (`crates/catalog/src/writers.rs`), split by `parts()` into `taken` and `taken_utc`.
- `Meta.frames` is a raw `u32` that reaches `Analysis.frames` and is bound in `store_analysis`. It is read back as `frames <= 1` and `coalesce(a.frames, 1) > 1` in the catalog and in `crates/query`'s media filters and sort key.
- Missed by the first draft: image `width` and `height` (`u32`, from the codec's header parse in `crates/codec/src/decode.rs`, zero not ruled out by the type), and video `width`, `height`, `duration_ms` (narrowed to a `NO_DURATION` sentinel) and `poster_ms`. All reach `store_analysis` raw.
- `Meta` reads no strings (no camera model, no GPS).

The readers' integers are wider than the first draft said. `Tiff::unsigned` (`crates/metadata/src/tiff.rs`) returns `Option<u64>` on purpose, since commit 16895197 (2026-11-23), so a BigTIFF LONG8 orientation is read at the width the patcher writes it. `Value::Number(u64)` and `Exif::rating()`/`rating_percent()` carry the `u64` inside the crate. The narrowing happens in the accessors: orientation by `u8::try_from` then `Orientation::new` (`exif.rs`), ratings by `min(5)`/`min(99)` then `u8::try_from` (`lib.rs`). XMP ratings are parsed as `f64` and narrowed by `xmp_stars`/`xmp_percent_stars` (`xmp.rs`). The `u64` stays `pub(crate)`: `Meta` exposes only `Orientation`, `Rating`, `i64` and `u32`.

Offsets: no slice index can panic (`indexing_slicing` is denied, every read is `.get`, `bytes_at` or `first_chunk`). Classic TIFF offsets go through `pg_model::num::index`. BigTIFF offsets and the EXIF IFD pointer use `usize::try_from(u64)`, and `pg-container` uses `usize::try_from(u32).unwrap_or(usize::MAX)`; both behave like `index`. Several offset additions (`tiff.rs` entry positions, the JPEG, PNG and GIF walkers) are unchecked but bounded by a buffer already in memory. There is no IFD-chain following, so a cycle cannot loop.

## Where to start

Domain types first, then the wrapper:

1. **`Frames(NonZeroU32)` in pg-model**, in the style of `RowHeight` (`crates/model/src/size.rs`): private field, `pub const ONE`, `pub const fn new(u32) -> Option<Frames>`, `pub const fn get`, `pub fn animated`, `TryFrom<u32>` with a `FramesError` that has `Display` and `core::error::Error`, `From<Frames> for u32`. No serde (`Analysis` has none).
   - `pg-container`'s `PngAnimation::frames` and `WebpAnimation::frames` return std's `NonZeroU32` (no new dependency).
   - `Container.frames`, `Meta.frames`, `Analysis.frames` and the video `Probe.frames` become `Frames`. GIF counting uses `saturating_add`.
   - `analyze.rs`'s border gate uses `!frames.animated()`. `store_analysis` binds `frames.get()`. Test fixtures change `frames: 1` to `Frames::ONE`.
2. **`Taken` with a private payload**: a range-checked constructor (`Taken::wall(i64) -> Option<Taken>`, the same for UTC) and accessors, so a match no longer yields a raw `i64`. `exif_date(data) -> Option<Taken>` and `Meta.taken: Option<Taken>`; drop the `.map(Taken::Wall)` in `analyze.rs`.
3. **`Untrusted<u64>` at `Tiff::unsigned`**, with the sealed `FromUntrusted` from option 1, producing `Orientation` and `Rating`. This is the first real use of the wrapper. pg-model is `no_std` with `alloc`, so the wrapper is `core`-only.
4. **The seal tests**: the `compile_fail,EXXXX` doctests listed in option 2.
5. **CodeQL** as a CI job, with the models above (option 3).

## Open questions

Each answer is the most proper option, not the cheapest. Churn is accepted.

### 1. Which conversion ladder?

Most proper: keep the `u64` step that exists, and make it the wrapped value. `Tiff::unsigned` returns `Untrusted<u64>`; `checked` narrows to `Orientation` or `Rating`. The narrowing is the validator, and it moves from the `Exif` accessors into the `FromUntrusted` impls.

- The container frame count is a `NonZeroU32` from `pg-container`, which becomes `Frames` (question 6).
- `exif_date` already builds its value from components checked by `wall_clock_seconds` (years 1–9999, real days), and its test `exif_dates_as_kamadak_read_them` feeds nine malformed dates and expects `None`. Its return type should be `Option<Taken>`.
- File offsets: the wrapper's offset method returns the bounds-checked sub-slice, not a `usize` (option 1).

### 2. Where does the wrapper live?

Most proper: `pg-model`, next to the domain types it exits to. `pg-metadata`, `pg-video` and `pg-yandex` already depend on `pg-model`. `pg-container` does not (its only dependency is crc32fast), so it returns std types (`NonZeroU32`) and `crates/metadata/src/container.rs` wraps them. pg-model is `no_std` with `alloc`, so the wrapper uses `core` only.

### 3. Which sources are untrusted?

Most proper: every external input. By risk:

1. **Yandex, a remote service.** The JSON's `pos` (`u32::try_from(...).ok()?`) and `width`/`height` (`try_from(...).unwrap_or(0)`) in `crates/yandex/src/parse.rs`, and above all its URLs, which are fetched and decoded (see "Found while checking").
2. **Video, through libavformat in-process** (no ffprobe process runs). The date tags `com.apple.quicktime.creationdate` (wall clock) and `creation_time` (UTC), parsed by `pg_model::date::parse_iso_date`, dates at or before 1970 dropped. `Stream::frames()` (`crates/video/src/ffi.rs`) floors `nb_frames` at 0 and narrows with `u32::try_from`, so a count above `u32::MAX` fails the probe; `probe` estimates from duration × frame rate when it is 0 and clamps to `1..=u32::MAX`. The display width `f64_to_u32(w * sar)` can reach `u32::MAX`.
3. **File containers and EXIF**: `crates/metadata` and `crates/container`. Also the image decoders' header parse (width, height).
4. **ExifTool's output**, a separate process, matched only with `contains` (`crates/exiftool`). Low.
5. **Old catalog files**: column and trigger names come from the app's own schema, but row values are copied raw with `INSERT … SELECT`, and views run (see `trusted_schema`). Thumbnail-cache JPEGs go back through the decoders. Low.
6. **Local config, command line, environment, the driver pipe**: `prefs.json` (a bad file is moved aside), `desktop/src/command_line.rs`, `pg_platform::env`, and `crates/driver/src/borders.rs` (bounded with `checked_mul` and a pixel cap; a dev tool, not shipped). Low.
7. **Values that go out to QML and come back** (question 4).

Not sources: the clipboard is write-only, there is no drag and drop, and file names keep their raw bytes (`Name::from_raw`).

Start with metadata, which has the most parsing. Yandex is the one remote source, and it should not wait behind the file work.

### 4. Does QML get the wrapper?

Most proper: no. A bridge call is an input boundary, and it converts at its entry. The real entry validator is `file_id_of(f64)` (`crates/desktop/src/bridges/rows.rs`), used by `shell`, `video_source` and `yandex`. `FileId::new` (`crates/model/src/ids.rs`) checks nothing.

The round trip is not hypothetical. File-derived strings do come back:

- **Extensions**: the extensions facet sends them out, and `filters.toggle` takes them back with no check at entry (`crates/client/src/input.rs`). They become an `Ext` code only when the query is built (`crates/query`), where an unknown one maps to -1.
- **Folder paths**: `keptRemovals` sends display paths out and `discard_removal` takes one back (`crates/desktop/src/library.rs`). Safe: matched against the catalog's own list.
- **Image keys**: `viewedKey` comes back through `fit_source`/`region_source` into `ImageKey::parse`, then `ImageRequest::parse`. Parsed, and fuzzed (`model_keys`).

Entries that do not check at entry: `set_file_id_and_play` (`animation.rs` stores the raw `i64`; the `<= 0` check comes later, in `restart`), `toggle` with extensions, `step_preview(by)`, and every `READ, WRITE` property with no custom setter (`gallery` `current`, `anchor`, `viewed_file_id`; `yandex` `preview_item`). Most proper: pass domain ids, check every entry at entry, and give writable properties a setter that validates.

`ext_of` (`crates/desktop/src/video_player.rs`) filters the extension it hands to `play_file` to ASCII alphanumerics (verified). The QML image provider never sees a remote URL; Yandex results reach it as `image://` ids.

### 5. What is the sink threat?

Most proper: decide the threat before the design. The threat is a semantically wrong value, not injection. An orientation of 9, a zero-frame animation, a zero width and an impossible date are what the domain types are for. (A rating of 7 is already rejected by the database: `ratings.rating` and `file_ratings.value` have `CHECK BETWEEN 1 AND 5`.)

Injection: the audit is done, and there is no path. Every value from the user or a file is a bound parameter. The only things written into SQL text are compile-time constants, enum-matched `&'static str` fragments, the app's own schema names and formatted integers:

- `crates/query`, the largest builder: `condition_without` joins `&'static str` fragments from `media_sql`, `shape_sql`, `status_sql` and `border_sql`. The name filter is bound to `pg_contains(f.name, ?)`, a case-insensitive substring match with no wildcards, so `%`, `_` and `"` need no escaping. No `LIKE`, `GLOB` or FTS `MATCH` is built from user text anywhere. Extensions become `Ext` codes, ranges are bound. `border_counts(cond: &str)` is `pub` and takes any `&str`, but its only caller passes `condition_without(..).sql`. The sort key comes from `key_sql`, `Option<&'static str>`.
- `crates/catalog/src/browse/read/grid.rs`, `facets.rs`, `scope.rs`: fixed text and constants, values bound.
- `dups.rs`: placeholder numbers from a loop index.
- `types.rs` `VolumeSet::and_in`, `work.rs`, `crates/thumbcache/src/reader.rs` `pending_sql`: column literals and `i64`/`u64` lists formatted as numbers.
- `queue.rs` `change_turns`: a `(&'static str, &'static str, &'static str)` tuple from literals.
- `convert.rs`: nothing is interpolated; schema and table are bound to `pragma_table_info(?1, ?2)`.
- `rebuild.rs`: table names from `CARRIED`; column names iterate the current schema (`main`), filtered by equality against the old file's list, so every name is the app's own (`copy_folder_rows` likewise, with literal value expressions). Trigger names come from `main.sqlite_schema` of the replacement. Identifiers are not quoted, which is harmless here. Both `ATTACH`es bind the path.
- `crates/dbfile/src/schema.rs`: `SELECT quote({column}) FROM main.{table}` with the caller's seeds, unquoted. `judge_conn` is `pub(crate)`. The public sink that takes non-static strings is `pg_dbfile::Schema::read(conn, &[(&str, &str)])`. Production callers: `Expected::schema` and `judge_conn` (through `Expected`, which takes `&'static` slices), and catalog and thumbcache `schema.rs`, which pass `SEEDS` (or `&[]`) directly. Safe today; nothing in the type stops a future caller passing runtime text.

Most proper fix for the dbfile gap: `&'static` alone is weak, since `Box::leak` of runtime text is `'static`. A `Seed` type with private fields, built only by `pub const fn Seed::new(table: &'static str, column: &'static str)` that panics at compile time unless both names match `[a-z_][a-z0-9_]*`; a `seed!` macro that expands to `const { Seed::new(..) }`; and `Schema::read`, `Expected::new` and `judge_conn` taking `&'static [Seed]`. Quote both identifiers as a second layer. Only the two dbfile tests that borrow a local array need changing.

Defence in depth: `baseline.sql` has CHECKs on ratings, `border_sides`, `file_writes`, `files.kind` and `session.id`. It has none on `analysis.orientation` (1–8), `frames >= 1`, `taken`, `width`/`height > 0`, `cut_*`/`duration_ms`/`poster_ms >= 0`, the `turns`/`rotation` columns (0–3), `files.ext`, the `missing`/`pending` flags, `file_failures.pass`, `ledger_ack.kind`, or hash lengths. Tradeoff: `copy_tables` treats a CHECK failure as fatal to the whole copy, so with a stricter CHECK one bad old row makes a rebuild carry nothing, not just drop that row. Add CHECKs together with a rebuild that skips or repairs bad rows.

### 6. Does a raw integer reaching a domain constructor matter?

Most proper: less than the probe made it sound. `Rating::new` and `TryFrom<u8>` already range-check. A raw `u8` that passes them is in range, whichever path it took.

Neither example the first draft gave needs the wrapper:

- XMP's `-1` ("rejected") is dropped by a float sign check (`(v >= 0.0).then(..)` in `xmp_stars`) before any integer exists. NaN becomes `None`; `+inf` saturates to 5. Side effect: "rejected" reads as absent, so the rating chain falls through to EXIF `Rating`. Whether a rejected XMP should mask an EXIF rating is a product question.
- Zero frames is already impossible on every path: the container walk starts at 1, PNG is `unwrap_or(1).max(1)`, WebP and GIF are `max(1)`, video clamps to `1..=u32::MAX`.

A domain type covers both. The wrapper matters where validity depends on the source, not the range; with the readers as they are, that is the narrowing in `Tiff::unsigned`'s callers.

`Rating` cannot be made file-only. The user's own ratings are built with it too: QML input in `crates/desktop/src/shell.rs`, the key map in `crates/client/src/controller.rs`, database reads in `grid.rs` and `writers.rs`, the job wire value in `crates/model/src/write.rs`, and the query filter. The only file-derived construction is in `crates/metadata/src/lib.rs`.

The real gap is `Taken`: a pub enum with pub variants, no validating constructor, built from any `i64`. It needs a private payload (see "Where to start").

The frame count needs its own domain type, `Frames(NonZeroU32)`, in the style of `RowHeight` (see "Where to start").

### 7. How is the seal kept?

Most proper: by the type structure and by tests in `pg-model`. The sealed `FromUntrusted` trait leaves no method that returns the value, and the `compile_fail,EXXXX` doctests (option 2) keep the seal from regressing. A `disallowed-methods` entry is added only if an accessor is ever added. Review rule: `FromUntrusted` is implemented only for pg-model newtypes with private payloads, and the wrapper never derives `PartialEq`, `PartialOrd`, `Hash` or `Default`.

### 8. What does the wrapper print, and what can it deserialize into?

Most proper: a hand-written `Debug` that prints `Untrusted(<unchecked>)`, and no derived `Debug`. A raw file value in a log is both a leak and noise. The validator's error is what belongs in a log. (Probed: a struct deriving `Debug` that holds the wrapper prints `Untrusted(<unchecked>)`.)

No `Serialize` and no `Deserialize` on the wrapper. Deserializing into `Untrusted` is the bypass the doc warns about (`secrecy` allows it; this plan does not). Domain types may keep `try_from` serde, as `Rating` does.

### 9. Plan or prototype?

Most proper: a plan first, in `docs/plans/`, next to `DECODE_ONCE_PLAN.md` and `BOUNDED_MEMORY_PLAN.md`. The plan fixes the decisions above (crate, seal, sources, sinks), because they decide where the churn lands across `metadata`, `container`, `video`, `yandex`, `analysis` and `catalog`. The prototype is done: the probe crate confirmed the sealed-trait design. Then implement the first slice (`Frames`, `Taken`, `Tiff::unsigned`). Build once at the end.

### 10. Which verification tool?

Most proper now: the tools already in the repo, plus CodeQL. `fuzz/` exists, and the test suites use `proptest` (in `app`, `client`, `container`, `jobs` and `model`). Fuzz the readers, because the parsers are the untrusted surface.

The existing targets in `fuzz/fuzz_targets/`:

| Target | Covers |
|---|---|
| `metadata_read` | `Meta::read` (EXIF, TIFF, XMP, the container) on arbitrary bytes |
| `container_chunks` | PNG, WebP and GIF walkers, `webp_exif`, `find` |
| `container_jpeg` | JPEG segments, `jpeg_exif`, progressive check |
| `edit_rotate` | `pg_edit::rotate::rotate`, which reaches `patch_orientation` with `UPRIGHT` only |
| `model_keys` | `ImageKey::parse` round trip |
| `model_query_json` | `Query` from JSON |
| `codec_transform` | the orientation transform only, not decoding |

Add, highest value first, before the wrapper lands so the refactor is checked against them:

1. **`yandex_parse`**: `parse_upload`, `parse_other_sizes`, `parse_similar_page` and `is_captcha_page` are already pure, public `&str -> Result` functions. Needs `pg-yandex` in `fuzz/Cargo.toml`. Checks: no panic, every `image_url` non-empty, sizes sorted.
2. **`codec_decode`**: the decode path Yandex downloads take. The most valuable missing target, given the URL issue.
3. **`model_date`**: `pg_model::date::parse_iso_date` is pure; today it is reached only through XMP dates in `metadata_read`.
4. **`video_facts`**: needs a refactor first. The frame, date, orientation and width logic sits inline in `probe` and in private helpers (`matrix_orientation`, `display_width`, `ticks_ms`), and `pg-video`'s build script needs FFmpeg. Move it into a pure function over the raw stream fields, in an FFI-free module. Checks: frames ≥ 1, orientation 1–8, no panic on `i32::MIN`/`i64::MIN`/NaN.
5. **`patch_orientation`**: every format × all 8 orientations; when it returns `Some(out)`, reading `out` gives back the orientation written. The writeback path (`crates/writeback`) with any orientation is not fuzzed today.
6. Lower: `ImageRequest::parse`, `Prefs` JSON, the desktop command line, `pg_driver::borders::serve`.

CodeQL (option 3) as a CI job for the source-to-sink check. Proofs later, for the few narrowing functions in pg-model (`percent`, `xmp_stars`, the `num.rs` conversions): Prusti natively on Windows first, since it needs no specifications; Kani only in the Linux container.
