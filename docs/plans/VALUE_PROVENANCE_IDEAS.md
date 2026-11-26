# Tracking values end to end: rationale

Status: rationale for `VALUE_PROVENANCE_PLAN.md` (the phased type work) and `SECURITY_HARDENING_FIXES.md` (the defects found on the way, which land first). Nothing here is built. Re-checked on 2026-11-26 in two research passes against the working tree, scratch probe crates outside the repo (the repo's toolchain, lints and SQLite build), and the tools' upstream sources. Each answer below is the most proper option, not the cheapest; churn is accepted. Cite by symbol; line numbers drift.

## The question

Can we track a value from where it enters the program (a file's bytes, a remote response, a catalog row, a QML argument) to where it is used (a SQL statement, an allocation, a path, a fetch, a QML property), the way Psalm's taint analysis tracks untrusted input to its sinks in PHP?

## The answer

Rust has no Psalm. The most proper substitute here is not a taint wrapper but **domain types at every boundary**: each reader parses straight into a type with a private payload and a validating constructor, so a raw value from outside never leaves the module that reads it. The catalog stores and reads those types through conversions that fail instead of repairing, the schema refuses what the types refuse, and SQL text can only be built from literals. rustc then enforces the boundary; tests keep the seals; fuzzing and exhaustive tests check the parsers and the narrowing; CodeQL scans for regressions.

The first draft's generic `Untrusted<T>` wrapper is dropped. Why is below (option 1).

## What Rust has

### 1. Types, enforced by rustc

**Domain types with private payloads** (chosen). A tuple constructor, a field read, a destructuring pattern and a private variant are all refused outside the defining crate (E0603, E0616; **probed**). Module privacy inside a reader crate keeps the raw read in: pg-metadata is `#![forbid(unsafe_code)]`, so privacy is as strong as any wrapper's seal.

**A generic `Untrusted<T>` wrapper** (rejected). The probes built one with the repo's toolchain and lints. Its seal holds: a downstream `.into()` (E0277), field access (E0616), `Display` (E0277), `*` (E0614) and `==`/`<` (E0369) are refused, and a sealed `FromUntrusted<T>` trait with a private token refuses downstream impls (E0277, E0603). But it is the wrong tool here:

- **Inside a reader crate it adds nothing.** The only code it would constrain is the narrowing code itself (`Exif` accessors, `Meta::read`).
- **One validator per target type cannot express the formats.** EXIF `Rating` clamps to 5 stars; `RatingPercent` clamps to 99 and maps to a band. Both arrive as `u64`; `FromUntrusted<u64> for Rating` cannot tell them apart. "Held but 0" must also win precedence.
- **A seal covers one crate.** pg-model cannot name another crate's private token, even through a `$crate::` macro (E0603); a `#[doc(hidden)] pub` seal is a convention a downstream crate can break. A wrapper crate sealed above pg-model can only call pg-model's public constructors, which adds nothing.
- **No value has to cross a crate boundary before it can be checked.** That is the one case a wrapper is for.
- **Wrapping the bytes** (briansmith's `untrusted`) protects nothing either: its taint ends at the first read (`Reader::read_byte` returns a plain `u8`), and every byte these parsers see is untrusted.

What the probes found about any wrapper, kept in case one is ever needed:

- A validator that takes a closure is an exit: `checked(|v| Ok(v))`, `checked(|v| Err(v)).err()`, and a closure that writes `v` into a captured variable all return the raw value.
- `PartialOrd` leaks the value by binary search, and `Hash` through a custom `Hasher`. Never derive them, nor `PartialEq` or `Default`.
- A `Debug` that prints the value is an exit (format, parse back). Having no `Debug` stops every holder from deriving it (E0277). So: a placeholder `Debug`.
- A validator error that carries the raw value hands back exactly the rejected values (`checked::<Orientation>().err()` returned 123456).
- `Copy` is safe; `#[must_use]` fires; derived serde on a holder fails to compile (E0277).
- If one is ever needed, it lives in pg-model with every impl beside it, implemented only for pg-model newtypes with private payloads, never for std types.

What types do not give: they do not show that every path to a sink passes a constructor. A public constructor such as `Rating::new(u8)` range-checks, so any in-range value is valid whatever its source; what matters is that each source has its own policy in one place (question 6).

### 2. Lints and tests that keep the boundary

The repo already denies `as_conversions`, `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `cast_lossless`, `cast_precision_loss`, `indexing_slicing`, `unwrap_used`, `expect_used` and `panic` (`Cargo.toml`, `[workspace.lints.clippy]`). All 11 production `as` casts are in `crates/model/src/num.rs`, each under `#[expect]`. `clippy.toml` already has `disallowed-types` (`HashMap`) and `disallowed-methods` (`temp_dir`, two `pg_platform::names` paths).

- `disallowed-methods` flags a method in all three forms (call, UFCS, fn value), crate-wide, including the defining module; it does not see field reads. An entry naming a method that does not exist warns on every run, so entries are added with the thing they ban. The plan uses it to ban the `&str`-taking rusqlite methods and `Connection::open*` (phase 6, fix 5).
- `compile_fail` doctests run under `just test` (only `crates/desktop` sets `doctest = false`), work in `no_std` crates, and are each a separate crate, which is exactly a downstream caller. Each must name its error code: this nightly enforces it, and a bare `compile_fail` passes on a typo. trybuild is not worth adding: its `.stderr` files change with nightly wording.
- `disallowed-macros` is crate-wide, so a "no `format!` into SQL" lint is not feasible; the `Sql` type does that job (plan phase 6).
- Dylint (v6.4.0, Windows in CI) is the optional mechanical check that no reader crate's public API holds a primitive integer (plan phase 8).

### 3. Dataflow tools and provers (measured)

- **CodeQL for Rust** (CLI v2.27.6, win64 bundle). On this repo with `build-mode=none` the database builds: 74 min here, 49 of them loading every Cargo project under the root (including `target/linux-head`), so scope it to `crates/`. The 19 cxx-qt bridge files do not expand; `platform/src/dir.rs`'s `offset_of!` does not parse; about 90% of calls resolve. `security-extended` gives 0 results. It models rusqlite's SQL sinks but not ureq. A custom query follows `dyn` dispatch, struct fields, `Vec` and iterator closures, but loses the flow at closures passed to other functions (`pg_jobs::panics::catch`, `read_similar_pages`), at `Mutex<HashMap>` state read on another thread, and at `params![..]`; it does not find the Yandex parse→`download` flow end to end, and summary models did not fix the closure break. So: a free regression scanner (the repo is public), not the proof that every path passes a validator. No results prove nothing.
- **Flowistry** (v0.5.44): "what influences this value" within one function, for the IDE. It does not analyse the bodies of called functions. Not a whole-program tool.
- **Paralegal** (Brown, on Flowistry): policy-based source and sink checking, but no Windows build, and it misses flows through `unsafe`, `RefCell` and `Mutex`.
- **MIRAI**: archived; the `endorlabs/MIRAI` fork is pinned to a January 2025 nightly.
- **Kani** (v0.68.0): Linux and macOS only (issue #1954). **Prusti** (nightly, Windows, Java 11): proves small copied functions and finds overflow, but cannot use std conversions without specs, and on pg-model it crashed on serde-derived code and did not finish in 45 min. Neither is used.
- **Exhaustive tests** are the proof for the narrowing functions: their domains are small. `round_half_away` against `f64::round` over all 2^32 f32 patterns: 0 mismatches in 89 s on one thread.
- Not for this question: Creusot, Verus, Flux, Charon/Aeneas, hax (specifications or rewrites); RAPx, Rudra, SafeDrop, MirChecker (memory safety, or dead); cargo-geiger (counts `unsafe`).

## Limits

- `unsafe` code and FFI are not seen by any of this: 267 lines under `crates/` open an `unsafe` block, function, impl or extern (video 106, platform 79, desktop 71, dedup 8, 3 in `catalog/examples`); cxx-qt generates more. `crates/metadata` forbids it.
- C parsers run in-process: libjpeg-turbo, libwebp, FFmpeg, SQLite. Types do not reach inside them; the fix list limits what reaches them (allocations, nested opens, SQLite settings). A sandboxed decode process for remote bytes is the strongest option and is left for later.
- QML is untyped JavaScript. A value's journey stops at the Rust receiver.

## Open questions, answered

### 1. Which conversion ladder?

Typed per-tag readers. `Tiff::unsigned` keeps returning `u64`, the lossless join of BYTE, SHORT, LONG and LONG8 (commit 16895197), visible only to `exif.rs`. Each tag's accessor returns its domain value, as `Exif::orientation` already does; the rating accessors return a `RatingField` with Windows' clamps applied there. Reader-native widths would need a four-width enum per tag. Plan phase 4.

### 2. Where do the types live?

Shared domain types in pg-model (`no_std`, `core` only), next to `Rating`, `Orientation` and `RowHeight`. A type that one crate both reads and checks lives in that crate: `ResultUrl` in pg-yandex. pg-container does not depend on pg-model and returns std types (`NonZeroU32`). Catalog conversions (`FromSql`/`ToSql`) live in pg-model behind an optional `rusqlite` feature, since the orphan rule rules out pg-catalog. If a wrapper is ever needed: pg-model, with every impl.

### 3. Which sources are untrusted?

Every input from outside the process, each parsed into a domain type where it enters. By risk:

1. **Yandex responses and result URLs**: fetched from any host and decoded (fix 3).
2. **Media bytes into the C decoders**, local or downloaded; header sizes reach allocations (fix 1).
3. **Video, through two libavformat contexts**: pg-video's and Qt Multimedia's (fix 4). The facts logic becomes a pure function (plan phase 0).
4. **The catalog file as a source of paths** for destructive writes (fix 2; SEC-015, SEC-016), and of rows for the rebuild (fix 5).
5. **Containers, EXIF, XMP** and the decoders' header parse.
6. **Directory listings**, whose buffer on a network drive is the server's answer (fix 6). Planned in `NTFS_MFT_SCAN_PLAN.md`: raw MFT and USN records parsed in an elevated helper, and its pipe in both directions, the highest-risk parsers once built. Every record and frame is bounds-checked, and the helper treats the parent's requests as untrusted.
7. **ExifTool's output** (fix 11).
8. **Old catalogs, the thumbnail cache, stored notices** (fix 5; notices already deserialize into `Notice`).
9. **Config, environment, command line, recorded sessions, the driver pipe**: `prefs.json` already deserializes into domain types (`RulePath`, `RowHeight`, `FileStamp`), the pattern the rest follow. SEC-028, SEC-034, SEC-056.
10. **QML arguments** (question 4).

Not sources: the clipboard (write-only), drag and drop (none), `QUrl` from QML (none), picker results (Rust keeps the `PathBuf`), live reload (debug builds, folder fixed at build time), FFmpeg DLLs (exe folder or `PG_FFMPEG_BIN`, never `PATH`).

### 4. Does QML get the wrapper?

No. Every bridge entry converts its arguments into domain types first. File-derived strings do come back from QML: extensions through `filters.toggle` (unchecked, and saved raw in `Query.exts`), folder paths through `discard_removal` (matched against Rust's own list), image keys through `fit_source`/`region_source` (parsed). The rules: no auto `WRITE` on a property QML does not write (63 of 64), validating setters on the rest, extensions become `Ext` at the bridge, ids refuse ≤ 0, floats are finite, `Fit` is clamped like regions. Plan phase 7, fix 9.

### 5. What is the threat?

**Attackers**, most likely first: a crafted media file on any scanned volume; Yandex's responses and the images they point to; a crafted `catalog.sqlite` or `thumbs.sqlite` (a restored or copied catalog); local environment, config and command line (same user, robustness only).

**Assets**: the user's originals (writeback and crop rewrite them), the catalog's user data (ratings, views, marks, queued writes), the network behind the machine, the process staying up.

**Must never happen**: code execution through a C parser or SQLite schema code; a write to a file the user did not choose; an abort (a panic, or an infallible allocation sized by an untrusted number); a wrong value stored as if valid; a fetch to a non-public host; a hang with no timeout. Out of scope: hiding the catalog from the same user, and slowness on large genuine files.

**SQL injection: none.** Every value from a user or a file is bound. Only literals, `&'static str` fragments, the app's own schema names and formatted integers reach SQL text: `crates/query` (`condition_without`, `border_counts`, `key_sql`), the catalog's `grid.rs`, `facets.rs`, `scope.rs`, `dups.rs`, `types.rs` `VolumeSet::and_in`, `work.rs`, `queue.rs` `change_turns`, `removal.rs`, `rebuild.rs` (names from `CARRIED` and from `main`'s schema, both `ATTACH`es bound), `convert.rs` (nothing interpolated), the thumbcache's `pending_sql`, and `dbfile`'s seeds. No `LIKE`, `GLOB` or FTS `MATCH` is built from user text; the name filter is `pg_contains(name, ?)`, with no wildcards. The gap is that the types do not enforce it (`Condition.sql` is a `pub String`, `Schema::read` takes any `&str`); plan phase 6 closes it.

The worst sinks are not SQL: header-sized allocations, stored paths, remote URLs, SQLite parsing a crafted file (fixes 1 to 5).

### 6. Does a raw integer reaching a domain constructor matter?

Range, no: `Rating::new` and `TryFrom` range-check, and stay public. Source, yes: one raw value means different things per source, and one lenient constructor serves them all today.

- **Catalog reads repair bad rows**: a stored rating of 7 reads as unrated, an orientation of 9 as upright, turns of 6 as 2. Corrupt rows are hidden, not reported. `ext_at`/`format_at` already do it properly. Fix: `FromSql` that errors (plan phase 5).
- **The 0-means-unrated rule is copied in five places**: one `Rating::from_stars` (plan phase 3).
- **File policy** (upright fallback, clamp to 5, percent bands) lives in the reader accessors only (plan phase 4).
- **XMP `-1` is a value, not an absence**: Bridge and darktable write it; digiKam reads it as none; Lightroom does not export rejects. The reader returns `Rejected`, and the first field held decides, so it masks a lower EXIF rating. To verify first: what Explorer shows.
- **Zero frames** is already impossible on every path; `Frames(NonZeroU32)` makes it a type.
- **`Taken` is the real gap**: a pub enum with pub `i64` variants. Plan phase 2 gives it private payloads, a range, and the UTC offsets that are dropped today.
- **Missed by the first draft**: image and video width and height, video duration (unknown stored as 0, fix 8) and poster, all raw. Plan phase 1.

`Rating` cannot be made file-only: the user's own ratings use it too.

### 7. How are the seals kept?

By privacy, checked by tests: each sealed type's `compile_fail` doctests with error codes (E0603 constructor, pattern and variant; E0616 field). Review rule: no reader crate's public API holds a raw integer read from input, except pg-container's buffer positions; Dylint can check it mechanically if review proves too weak. `#[non_exhaustive]` adds nothing on top of a private field.

### 8. What do the types print, and what can they deserialize into?

Domain types and their errors print freely: file values are not secret, and an error carrying the rejected value (`RatingError::TooManyStars`) helps diagnosis. Domain types keep `try_from` serde, as `Rating` does, so deserializing validates. Untrusted text shown or logged goes through one `display_text()` that removes control and bidi-format characters (fix 10).

### 9. Plan or prototype?

The prototype is done (the probe crates). Split into two documents: `SECURITY_HARDENING_FIXES.md`, a fix list in the style of `BACKGROUND_JOB_AUDIT_FIXES.md`, linked to its SEC ids, which lands first; and `VALUE_PROVENANCE_PLAN.md`, phased, whose phase 0 pins current behavior with tests before any refactor, checked once at the end in the repo's usual way. This file stays as the rationale.

### 10. Which verification tools?

No new prover. In order: rustc types and `compile_fail` doctests; the existing clippy lints plus the plan's bans; exhaustive or property tests for the narrowing functions; cargo-fuzz for the byte parsers, with round-trip checks (`patch_orientation`, `yandex_parse`, `codec_decode`; seeds generated at run time from pg-codec's fixture encoders and the ExifTool clone's `t/images`, none committed; runs natively on Windows with ASan). Video facts and dates get proptest, not fuzzing. CodeQL as a CI regression scanner. Prusti and Kani: no (option 3).
