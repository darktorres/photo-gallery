# Domain types at every boundary: phased plan

> **Status (2026-11-26).** Nothing here is built. The rationale, the alternatives rejected and the probe results are in `VALUE_PROVENANCE_IDEAS.md`. The defects found on the way are in `SECURITY_HARDENING_FIXES.md`, which lands first and does not wait for this plan. Claims are marked **read** (in the working tree of this date), **probed** (scratch crate, repo toolchain), or **to verify** (the check is named).

## Goal

No raw integer, float or string read from outside the process crosses a crate boundary. Every reader returns domain types whose payload is private and whose constructor validates. The catalog stores and reads those types through conversions that fail on a bad value instead of repairing it, and the schema refuses what the types refuse. SQL text can only be built from literals. Bridge entries convert their arguments before anything else.

Churn is accepted. The most proper option is chosen at each step.

## Ground rules

- Cite by symbol; line numbers drift.
- Crate layers: `tools/crate-layers.txt` lists every internal edge and `tools/boundaries.py` checks it. Phase 5 adds an optional `rusqlite` dependency to pg-model; edit the layers file in the same commit if it records optional edges.
- pg-model is `no_std` with `alloc` (except under the `specta` feature). New types use `core` only.
- Each domain type follows `RowHeight` (`crates/model/src/size.rs`): private field, `const fn new -> Option`, consts for the bounds, `TryFrom` with an error enum that has `Display` and `core::error::Error`, `From<T> for` the primitive, serde `try_from`/`into` where the type is serialized.
- Each domain type that seals something gets `compile_fail` doctests with error codes. The codes measured on the repo's nightly: tuple constructor E0603, field read E0616, destructuring pattern E0603, private enum variant E0603. A wrong code fails the test (**probed**). pg-model has no doctests today; these are its first, and `just test` runs them.
- A schema change follows the catalog's version and migration rules (`DATABASE.md`, `crates/model/src/versions.rs`).
- Process: one phase at a time, commit on `main`, no branches, commit only the phase's files. Phase 0 is checked at its end, because it is the baseline the rest is measured against. Phases 1 to 8 are checked once, at the end (Phase 9).

## Phase 0: pin current behavior

Before any type changes, so the refactor is checked against it.

1. **Fuzz targets**, in `fuzz/`:
   - `patch_orientation`: for each orientation 1–8, when `patch_orientation(data, format, o)` returns `Some(out)`, then the orientation read from `out` is `o`, the length is unchanged and `Meta::read(out)` succeeds. Pure, no new dependency.
   - `yandex_parse`: `parse_upload`, `parse_other_sizes`, `parse_similar_page`, `is_captcha_page` on raw text, with a dictionary (the JSON keys, `data-state`, `ImagesApp-`, `&quot;`) and seeds from the `parse.rs` tests. Checks: no panic, sizes sorted. Not "every `image_url` non-empty": `/s?img_url=` gives an empty one today (**probed**). Once `ResultUrl` exists, every URL must build one. Adds pg-yandex (ureq, rustls, ring) to `fuzz/Cargo.lock`; update `fuzz/deny.toml`.
   - `codec_decode`: `Format::sniff`, then `Decoder::decode_into` with `MinSize::Full` and `Sides`. Run with `-rss_limit_mb=2048` or more. The C and C++ JPEG decoders are not instrumented, so the fuzzer does not see inside them.
   - Seeds are generated at run time from pg-codec's `fixture` encoders and read from the ExifTool clone's `t/images` (`D:\GitHub\exiftool`, tag 13.64); none is committed. `fuzz/corpus` does not exist today, though `fuzz/Cargo.toml` says it does; fix the comment.
   - cargo-fuzz runs natively on Windows with ASan once MSVC's `clang_rt.asan_dynamic-x86_64.dll` folder is on `PATH` (**probed**), and in the container (`just fuzz-smoke`).
2. **Exhaustive tests** for the narrowing functions, in place of a prover: `round_half_away` against `f64::round` on all 2^32 f32 bit patterns widened, plus a sweep of f64 inputs around every half-integer and exponent boundary (0 mismatches in about 2 min on one thread, **probed**); `xmp_stars` and `xmp_percent_stars` stay ≤ 5 on the same inputs; `stars_of_percent` on all 256 inputs.
3. **The video facts as a pure function.** Move the frame, date, orientation, display-width and duration logic out of `probe` and the private helpers (`matrix_orientation`, `display_width`, `ticks_ms`) into an FFI-free function over the raw stream fields (sizes, aspect ratio, `nb_frames`, duration, time base, frame rate, matrix, the two date tags), with its own rational type. Test it with proptest: frames ≥ 1, orientation 1–8, no panic on `i32::MIN`, `i64::MIN`, NaN, all eight orientation matrices. Not a fuzz target: its input is about ten numbers.
4. **Dates**: a proptest round trip for `pg_model::date` (`parse_iso_date`, `wall_clock_seconds`).

## Phase 1: counts, sizes and lengths

- `Frames(NonZeroU32)` in pg-model: `ONE`, `new`, `get`, `animated`, `TryFrom<u32>` with `FramesError::Zero`. pg-container's `PngAnimation::frames` and `WebpAnimation::frames` return std's `NonZeroU32` (pg-container does not depend on pg-model, and needs no new edge). `Container.frames`, `Meta.frames`, `Analysis.frames`, the video facts use `Frames`. The border gate in `analyze.rs` uses `!frames.animated()`.
- `Size { width: NonZeroU32, height: NonZeroU32 }` for `Analysis`, the video facts, pg-codec's `Info` and `Dims` and `animation::Frame`, and the Yandex `OtherSize`. `Entry` holds `Option<Size>` (8 bytes through the niche). A Yandex `OtherSize` of zero is skipped; `SimilarItem` holds `Option<Size>`.
- `DurationMs` and `PosterMs`. Unknown length is `None` (fix 8 in the fix list). `Entry` keeps 4 bytes by storing `!ms` in a `NonZeroU32`, so `Option<DurationMs>`'s niche is `u32::MAX`; lengths saturate at `u32::MAX - 1`. `PosterMs` is built only from `Option<DurationMs>` (`min(0.1 × duration, 10 s)`), so it cannot pass the duration. One `Option<PosterMs>`, for videos only, replaces the three spellings of "no poster" (0 in `PlaceholderWork`, `None` in `FileSource`, `Some(0)` for an unanalysed video in `state.rs`).
- `encode::jpeg_quality` returns a quality type (1–100), not `u8`.

## Phase 2: Taken

```rust
pub enum Taken { Wall(WallClock), Zoned(WallClock, UtcOffset), Utc(Instant) }
```

- `WallClock(i64)`: seconds of the proleptic civil clock, years 1–9999, built from components (today's `wall_clock_seconds`) or by a range-checked `from_seconds` for catalog reads. `Instant(i64)`: same range. `UtcOffset(i32)`: within ±18 h. Payloads private. No date crate is added; `date.rs` already does the civil arithmetic.
- `parse_iso_date` returns `(WallClock, Option<UtcOffset>)` instead of dropping the offset. `Utc` is built as wall minus offset, so a `creation_time` that is not `Z` is no longer mislabeled. QuickTime's `creationdate` `+hhmm` and EXIF `OffsetTimeOriginal` (not read today) produce `Zoned`.
- Pre-1970 dates are kept (scans, edited EXIF). Only the MP4/QuickTime reader drops exactly epoch zero (1970-01-01T00:00Z is the muxer's "no clock"; FFmpeg already skips the 1904 zero). Camera reset defaults such as 2000-01-01 cannot be told from real dates and are kept.
- `exif_date` returns `Option<Taken>`; `Meta.taken` is `Option<Taken>`; drop the `.map(Taken::Wall)` in `analyze.rs`.
- Catalog: `taken` holds wall seconds (filters and sorts use it); `taken_utc` holds the instant whenever it is known (`Zoned` or `Utc`); a new `taken_offset` column holds the offset of `Zoned`. `set_taken_zone` recomputes only rows where `taken_utc IS NOT NULL AND taken_offset IS NULL`. The local conversion is `pg_localtime` (fix 7).

## Phase 3: orientation, turns, rating

- `Orientation`: an 8-variant `#[repr(u8)]` enum (`Upright = 1 … Rotate270 = 8`), the dihedral group D4, with full composition `then(Orientation)`, `compose(Turns)` as the special case, and `From<Turns>`. No wildcard match arms (`parts`, `source_pixel`, `shown` have them today and hide bad values). The upright fallback for an unknown tag lives only in the EXIF and display-matrix readers; public `Orientation::from_tag` goes.
- `Turns`: a 4-variant enum (C4). It stays a separate type, so `files.turns` and the write queue cannot hold a mirror. The mod-4 `Turns::new(u8)` becomes `-> Option`; `Turns::of(i64)` stays for signed user turns.
- `Rating`: an enum or `NonZeroU8`, so `Option<Rating>` is one byte. `Rating::new` and `TryFrom` stay public: they range-check, and every in-range value is a valid rating whatever its source. One `Rating::from_stars(u8) -> Result<Option<Rating>, RatingError>` (0 is `Ok(None)`) replaces the copies of the 0-means-unrated rule in `shell.rs` (twice), `controller.rs`, `write.rs` `from_stored` and `query.rs` `RatingFilter`.

## Phase 4: the readers return domain types

The public boundaries that carry raw file or remote values today (**read**), and what each becomes:

| Crate        | Today                                                                                                                   | Becomes                                                                                                                 |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| pg-metadata  | `Meta.taken: Option<i64>`, `Meta.frames: u32`                                                                           | `Option<Taken>`, `Frames`                                                                                               |
| pg-container | `PngAnimation::frames`, `WebpAnimation::frames -> u32`; `GifImage` `u16` sizes; `ExtendedChunk.full/offset: usize`      | `NonZeroU32`; a checked GIF frame rectangle; the chunk positions stay `usize` (bounded by the buffer, read with `.get`) |
| pg-codec     | `Info.full_width/full_height/scale_denom`, `Dims`, `animation::Frame` size and `delay_ms`, `jpeg_quality -> Option<u8>` | `Size`, a scale enum, a delay type, the quality type                                                                    |
| pg-video     | the probe's width, height, duration, frames; `Taken` with a public payload                                              | the Phase 0 facts function's domain types                                                                               |
| pg-yandex    | `OtherSize`/`SimilarItem` sizes and `pos`; URLs as `String`                                                             | `Size`/`Option<Size>`; `pos` stays `u32`, shown with `checked_add`; `ResultUrl`                                         |
| pg-model     | `Analysis` width, height, frames; `VideoFacts` duration and poster; `Taken::Wall/Utc(i64)`                              | Phases 1 and 2                                                                                                          |

In pg-metadata:

- `tiff` moves under `exif`; `Tiff::unsigned`, `Value::Number(u64)` and `number` become `pub(super)`. `u64` stays: it is the lossless join of BYTE, SHORT, LONG and LONG8 (commit 16895197), and the patcher in `orientation.rs` does not use it.
- Each tag's accessor returns its meaning. `Exif::orientation() -> Orientation` already does. `Exif::rating()` and `rating_percent()` stop returning `u64` and return a `RatingField`, with Windows' clamps applied there; `xmp_stars` and `xmp_percent_stars` do the same. `lib.rs` only chains the four fields.
- `enum RatingField { Unrated, Rejected, Stars(Rating) }`. The first field the file holds decides, in Windows' order: Microsoft percent, EXIF percent, `xmp:Rating`, EXIF `Rating`. So a held `xmp:Rating = -1` masks a lower EXIF rating, where today it reads as absent and falls through. The catalog has no reject state: `Rejected` maps to no stars, and `Rating` stays 1–5. **To verify first:** what Explorer shows for a synthetic file with `xmp:Rating=-1` and EXIF `Rating=4`; the repo's rule is "as Windows shows it", and the order follows that measurement.
- `ContentBox` (checked `l ≤ r ≤ width`, `t ≤ b ≤ height`) and a derived `Cut`; `Borders::Found(Cut)`. Replaces the unchecked `Detection::cut`.

## Phase 5: the catalog stores and reads domain types

- `FromSql` and `ToSql` for every pg-model type the catalog stores (`Rating`, `Orientation`, `Turns`, `Ext`, `Format`, `Frames`, `Size` parts, `DurationMs`, `PosterMs`, `WallClock`, `Instant`, `UtcOffset`, `FileHash`, `FileId`), behind an optional `rusqlite` feature in pg-model. The orphan rule rules out pg-catalog; the `specta` feature is the precedent. The web build leaves it off. `FromSql` returns `OutOfRange` or `InvalidType` and never repairs. No such impls exist today.
- This replaces the reads that hide bad rows today (**read**): `grid.rs` `entry_from_row` reads a stored rating of 7 as unrated (`.and_then(Rating::new)`), an orientation of 9 as upright (`from_tag`), turns of 6 as 2 (`Turns::new`); the same in `writers.rs` `known_rating`, `lookup.rs`, `state.rs`, `work.rs`. `types.rs` `ext_at` and `format_at` already error, and move into `FromSql`.
- Every table becomes `STRICT` (the baseline's column types already fit; without it `'abc'` sits in an INTEGER column, **probed**). The CHECK constraints:
  - `analysis`: `version >= 0`, `size >= 0`, `rotation BETWEEN 0 AND 3`, `kind IN (0,1)`, `orientation BETWEEN 1 AND 8`, `width` and `height` and `frames BETWEEN 1 AND 4294967295`, `format BETWEEN 0 AND 255`, `pdq_quality BETWEEN 0 AND 100`, `length(file_hash) = 32`, `length(pixel_hash) = 16`, `length(pdq_dihedral) = 128`, `taken BETWEEN -62135647200 AND 253402351200` (years 1–9999, leap second, ±14 h), `taken_utc BETWEEN -62135596800 AND 253402300800`, `taken_offset BETWEEN -64800 AND 64800`, `duration_ms BETWEEN 0 AND 4294967294`, `poster_ms >= 0`, `cut_* >= 0`, `borders_due IN (0,1)`. Row shape: `(error IS NULL) = (orientation IS NOT NULL AND width IS NOT NULL AND height IS NOT NULL AND frames IS NOT NULL)`; `kind = 0 OR format IS NULL`; `kind = 1 OR (codec IS NULL AND duration_ms IS NULL AND poster_ms IS NULL)`; `taken_utc IS NULL OR taken IS NOT NULL`; the four `cut_*` all NULL or all set.
  - `files`: `ext BETWEEN -1 AND 255`, `size >= 0`, `missing`, `pending`, `unpaired IN (0,1)`, `turns BETWEEN 0 AND 3`, `seq >= 0`, `changed >= 0`.
  - `placeholders`: `rotation BETWEEN 0 AND 3`, `size >= 0`, `version >= 0`. `file_failures`: `pass IN ('thumbs','borders','import')`, `rotation BETWEEN 0 AND 3`, `hidden IN (0,1)`, `retry >= 0`. `ledger_ack`: `kind IN ('thumbs','borders','import')`. `folder_failures`: `reason IN ('unreadable','mount-refused','refused')`. `notices`: `level BETWEEN 0 AND 2`, `count >= 1`. `pdq_pairs`: `a < b`, `distance BETWEEN 0 AND 256`. `border_sides`: `pass >= 0`, `depth >= 0`, `color BETWEEN 0 AND 16777215`, `rejected BETWEEN 0 AND 5`. `views`, `session`: `recursive IN (0,1)`. `root_removals`: `deferred IN (0,1)`. `meta`: the counter keys (`seq`, `import_version`, `borders_version`, `thumbs_version`, `pairing_epoch`, `pairing_version`, `vacuum_owed`) hold a non-negative integer (a `seq` past `i64::MAX` silently becomes REAL, **probed**).
  - Each code range (`ext`, `format`, `rejected`, the `pass` names) gets a test that it matches its Rust enum, as the `pairing_version` seed test does. **To verify:** the `retry`, `poster_ms` and `cut_*`-against-`width` semantics before adding those three.
- **The rebuild copies what it can.** Today a CHECK failure is fatal to the whole copy (`copy_tables`), so one bad old row would carry nothing. Instead (**probed**, `sqlprobe` `carry.rs`): try `INSERT … SELECT` per table; on a constraint error that statement alone rolls back, and the table is copied row by row with one prepared insert, each refused row skipped and counted by extended code (275 CHECK, 3091 type, 787 foreign key, 1555/2067 key). A child of a skipped parent fails its foreign key and is skipped too. `Carried::Rows` gains the refused counts per table and constraint, shown in the rebuild notice. Values are never repaired. After the copy, recompute the flags that skipped rows leave stale (`files.pending`, `hashes_owed`, `pairs_owed`, `borders_due`). `INSERT OR IGNORE` is not used: it also drops NOT NULL and UNIQUE failures silently, does not skip foreign-key failures, and on a `STRICT` table a type error aborts the statement (**probed**).

## Phase 6: SQL text from literals only

The injection audit found no path (`VALUE_PROVENANCE_IDEAS.md`, question 5). This phase makes the types enforce it.

- `Sql` with a private field, built only by `sql!(...)`, which expands to `const { Sql::literal(concat!(...)) }`. The `const` block is what keeps runtime text out; leaked runtime text cannot reach it. Runtime builders: `Sql::push(&Sql)`, `Sql::placeholders(n)`, `Sql::integer(i64)`.
- `Ident`: `[a-z_][a-z0-9_]*`, checked at runtime too (a `const fn` can also run at runtime), always written quoted with `"` doubled. Used by `rebuild.rs` (table, column and trigger names; `DROP TRIGGER main."{name}"` does not escape `"` today) and `dbfile`.
- `Condition { sql: Sql, params }` with private fields (today `Condition.sql` is a `pub String`). `border_counts(&Sql)`, `VolumeSet::and_in(Ident)`.
- dbfile seeds: declared once on `Expected` (`seed!(meta.key)` gives `Seed { table: Ident, column: Ident }`). `Schema::read` becomes `pub(crate)` and takes its seeds from `Expected`, so no caller passes seeds. The catalog and thumbcache callers that pass `SEEDS` directly call the `Expected` method instead; the two dbfile tests that borrow a local array change.
- `clippy.toml` `disallowed-methods`: the `&str`-taking rusqlite methods (`Connection::prepare`, `prepare_cached`, `execute`, `execute_batch`, `query_row`, `query_one`, `pragma_update` and the rest), so all SQL goes through an extension trait taking `&Sql`. `Transaction` reaches `Connection` through `Deref`, so it is covered. Also `Box::leak`, `String::leak`, and the hidden `Sql::literal`. **To verify:** whether clippy flags `Sql::literal` when it comes from the `sql!` expansion; if it does, the macro calls a differently named hidden constructor that is not banned.

## Phase 7: bridge entries convert first

- **No auto `WRITE` on a property QML does not write.** 63 of the 64 auto-setter properties are never written from QML (**read**). They become `READ, NOTIFY`; Rust writes them through a private helper (`rust_mut()` plus the `*_changed()` signal), since cxx-qt 0.10 generates `set_<name>` only for an auto `WRITE`. The properties QML does write (`anchor`, `current`, `viewing`, `previewItem`, `layoutWidth`, `gap`, animation `fileId` and `playing`, settings, `facet`) use `WRITE = setter`, and the setter validates. `gallery.current` and `yandex.previewItem` have custom setters today that do not validate.
- **Ids**: `FileId::new` returns `Option` and refuses ≤ 0; so does `ImageKey::parse`. `indexOf` and `viewedFileId` convert at entry.
- **Strings become enums at the bridge.** `Filters.toggle` turns an extension into `Ext` with `Ext::of` and ignores an unknown one. `Choice::Ext(Ext)` and `Query.exts: Vec<Ext>` (serde by key with `try_from`), so the `-1` code in `crates/query` goes away and no raw extension string is saved in sessions or views.
- **Numbers become checked types**: finite positive floats for `relayout` and the Viewport inputs (fix 9 refuses bad values; this phase gives them types).
- File-derived strings that come back (`discard_removal`, `viewedKey`) stay as they are: matched against Rust's own list, or parsed by the fuzzed `ImageKey::parse`.

## Phase 8: keep it from regressing

- The `compile_fail` doctests for each sealed type (ground rules).
- **CodeQL** as `.github/workflows/codeql.yml`: on push to main, pull requests and weekly; `security-events: write`; `github/codeql-action` init and analyze pinned by commit SHA, as the other workflows are (`v4` is an annotated tag; dereference it); `languages: rust`, `build-mode: none`, `queries: security-extended`; a config with `paths: [crates]`; the pinned nightly from `rust-toolchain.toml`; FFmpeg headers and libclang for pg-video's bindgen build script. A model pack: the Yandex parsers as a remote source, `Fetcher::download` as a request-URL sink. It is a regression scanner, not proof (`VALUE_PROVENANCE_IDEAS.md`, option 3). The repo is public, so code scanning is free.
- A scheduled CI fuzz smoke run (today `just fuzz-smoke` runs only in the container).
- Optional, if review proves too weak: a Dylint lint that no `pub` field or return value in a reader crate (pg-metadata, pg-container, pg-codec, pg-video, pg-yandex) has a primitive integer type, except pg-container's buffer positions.

## Phase 9: validate

One `cargo check`, then `cargo test --workspace --locked`, `just lint`, the doctests, a fuzz smoke run of the Phase 0 targets, and a rebuild of a catalog from the previous schema with one deliberately bad row per constrained table (each refused and counted, the rest carried). Fixes found here go in a closing commit.
