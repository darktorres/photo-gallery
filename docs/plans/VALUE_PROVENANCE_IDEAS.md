# Tracking values end to end: ideas for later

Status: ideas only, not planned work. Nothing here is implemented, and no tool named below is installed in this repo's toolchain. The facts below were checked against the repo and a probe crate (outside the repo) on 2026-11-13.

## The question

Can we track a value from where it enters the program (a file's bytes, a QML property) to where it is used (a SQL bind, a slice, a QML property), the way Psalm's taint analysis tracks untrusted input to its sinks in PHP?

## What Rust has

Rust has no built-in taint analysis. These are the closest options, in the order to try them.

### 1. Types, enforced by rustc (the most proper option here)

Wrap values that come from outside the program in a type with private fields and no `Deref`, `Into` or `From`. Its only way out is a validator that returns a domain type.

- Values read from a file enter as `Untrusted<T>` at the reader (`crates/metadata/src/tiff.rs`, `exif.rs`, `xmp.rs`, `container.rs`).
- The only exit is `checked`, which takes a validator and returns a domain type: `Rating`, `Taken`, a frame count, an `Orientation`.
- Sinks take domain types only: SQL binds, slice indexes, QML properties.

What the probe confirmed: a downstream `.into()`, field access, `Display`, and `*` on the wrapper are all rejected by rustc.

What it does not give:

- **Domain constructors take raw integers.** `Rating::new(u8)` and `Rating`'s `TryFrom<u8>` are public. A raw `u8` can become a `Rating` without touching the wrapper. The wrapper constrains only the values that are wrapped. The sink is safe only where no raw integer reaches a domain constructor.
- **`Debug` prints the raw value.** A derived `Debug` on the wrapper is a log leak, not a sink, but it is an exit.
- **A `pub` accessor defeats the seal.** Only a lint or review stops one, and rustc does not.

So rustc checks the wrapper's seal. It does not check that every path to a sink passes a validator. That needs the lint in option 2.

### 2. Lints that enforce the boundary

The repo already denies `as_conversions`, `cast_possible_truncation`, `cast_sign_loss` and `cast_possible_wrap` (`Cargo.toml`, in `[workspace.lints.clippy]`). All 11 production `as` casts sit in `crates/model/src/num.rs`, each under `#[expect]`, so no silent `as` cast sits between the layers.

`clippy.toml` already uses `disallowed-types` (for `HashMap`) and `disallowed-methods` (for `temp_dir`). The same mechanism can ban the wrapper's escape hatch. The probe checked that clippy flags the `into_inner` escape in three forms: a method call, a UFCS path call, and passing it as a fn value. Two limits:

- **It is crate-wide.** A call from inside the wrapper's own module is flagged too. So the wrapper has no method that returns the value: `checked` reads the private field (`self.0`). The lint does not flag a field read, even inside the wrapper's module (probed), so it needs no exemption.
- **It does not see fields.** A `pub(crate)` field read is not flagged. The wrapper's fields must stay private, and that is enforced by rustc, not by the lint.

Add the `disallowed-methods` entry together with the wrapper type, not before.

Negative tests belong with the wrapper too. A `compile_fail` doctest for each forbidden trait (`Into`, `Display`, `Deref`, field access) keeps the seal from regressing without relying on review.

### 3. Whole-program dataflow and proofs

- **MIRAI** is archived (last push 2024-08-22). Do not build on it.
- **Flowistry** is active (last push 2026-05-22). Its README describes an information-flow analysis: it tells whether one piece of code can affect another, and an IDE "focus mode" shows the code related to a chosen variable. It has no source-and-sink policy, so it answers "what influences this value?", not "does untrusted input reach this sink?". Its README lists interior mutability as a known gap.
- **Kani**, **Prusti** and **Creusot** are active upstream. They can prove a property of a single function, such as "this conversion never truncates". None is installed, and none has been tried on repo code.

## Limits of all of these

- `unsafe` code and FFI are not seen. A grep of the current tree finds 274 source lines under `crates/` that contain `unsafe` (lines, not the 472 lint hits in `CLIPPY_LEDGER.md`); they open an `unsafe` block, function, impl or extern, including the cxx-qt bridge. `crates/metadata` has none.
- QML is untyped JavaScript. A value's journey stops at the Rust receiver.
- A `serde` impl can build a domain type straight from input. That is safe only when the impl goes through the validator. `Rating` does (`#[serde(try_from = "u8")]`).
- A derived `Serialize` on the wrapper would be a leak route. Not probed here; keep it off the wrapper.

## What the repo carries today

The file-derived values that reach the database raw are the gap, not the ones the first draft named:

- `Meta.orientation` is already an enum, and `Meta.rating` is already `Option<Rating>`.
- `Meta.taken` is `Option<i64>`, wrapped as `Taken::Wall(i64)`. The inner `i64` is raw, and it reaches the catalog writers (`crates/catalog/src/writers.rs`, around line 307).
- `Meta.frames` is a raw `u32` that reaches the `Analysis` record and the catalog writers (`writers.rs`, around line 326).

The readers' integers are narrower than the first draft assumed: `tiff.uint` returns `u32`, `xmp_value` returns `u8`, and `exif_date` returns `i64`. No `u64` crosses the metadata boundary today.

## Where to start

Option 1 at the readers in `crates/metadata`, not at `Meta`:

1. `tiff.uint`, `exif_date` and the container's frame count return `Untrusted<_>` at their native widths.
2. `checked` produces `Taken` and a frame-count domain type (a `NonZero` or a small newtype, since the count is `max(1)`).
3. Option 2 keeps the boundary from regressing: the `compile_fail` tests for the seal, and a `disallowed-methods` entry for any `into_inner` added later.

## Open questions

Each answer is the most proper option, not the cheapest. Churn is accepted.

### 1. Which conversion ladder?

Most proper: the reader returns the width it already parses, wrapped as `Untrusted`. The validator returns a domain type from `pg-model`. No intermediate `u64`.

- `tiff.uint` returns `Untrusted<u32>`, which then becomes `Orientation` or `Rating`.
- The container's frame count is `Untrusted<u32>`, which becomes a frame-count type (question 6).
- `exif_date` already computes its value from parsed components, and its tests feed it malformed dates and expect `None`. Its return type should be `Option<Taken>`. The wrap belongs on the bytes it reads.
- File offsets (EXIF IFD pointers, TIFF entry offsets) are untrusted `u32`s that index the byte slice. Their validator is the bounds check that `bytes.get` already does, returning `usize` through `pg_model::num::index`.

Why no `u64`: `tiff.uint` already narrows with `u32::try_from` on its LONG8 path, so the narrowing is the validator. A `u64` step adds a width nothing needs.

### 2. Where does the wrapper live?

Most proper: `pg-model`, next to the domain types it exits to. `pg-metadata` already depends on `pg-model`, and so do `crates/video` and `crates/yandex` (checked in their `Cargo.toml` files). No new dependency is needed for any of the three readers.

### 3. Which sources are untrusted?

Most proper: every external input, not only the metadata crate. Four sources exist in the repo today:

- File containers and EXIF: `crates/metadata` and `crates/container`.
- ffprobe output for video: the `creationdate` and `creation_time` tags (`crates/video/src/lib.rs`, around line 140), and the frame count, which is an `i64` from `stream.frames()` (`crates/video/src/ffi.rs`).
- The Yandex similar-images JSON, a remote service: `pos`, `width` and `height` are narrowed with `u32::try_from` (`crates/yandex/src/parse.rs`, lines 153 and 159 to 160).
- Strings that came from a file, went out to QML, and came back through a bridge call (question 4).

Start with metadata, which has the most parsing. Then video and yandex, which have the same shape. Yandex is the one remote source, and it should not wait behind the file work.

### 4. Does QML get the wrapper?

Most proper: no. QML is not an input source. Its values come from the app's own model, or from files (already covered). A bridge call is an input boundary, and it converts at its entry the way `crates/desktop/src/animation.rs` does with `FileId::new(file_id)`.

The round trip is hypothetical in this repo: I found no file-derived string that comes back from QML. The one file-derived string that crosses a bridge goes out, not in. `ext_of` (`crates/desktop/src/video_player.rs`) hands the extension to `play_file`, filtered to ASCII alphanumerics. If a round trip ever appears, pass the domain id across the bridge (a `FileId`, a `Rating`) and let Rust look the value up, rather than send the file's string back.

### 5. What is the sink threat?

Most proper: decide the threat before the design. The threat is a semantically wrong value, not injection. A rating of 7, an orientation of 9, a zero-frame animation and an impossible date are what the wrapper is for.

Injection is a smaller question, and the audit is done. Values are bound parameters (`?1`, `?2`). The production SQL built with `format!` interpolates only constants and schema names, and no file-derived value:

- `crates/catalog/src/browse/read/grid.rs`: `scope` is fixed text with bound values. The sort key comes from `key_sql`, which returns an `Option<&'static str>`.
- `facets.rs` and `dups.rs`: fixed text, with values bound.
- `convert.rs`: the schema name comes from the `Source` enum.
- `rebuild.rs`: table names come from the `CARRIED` constant. Column names are the intersection of the current schema's columns (`main`) with the old file's, so every name is one the app's own schema defines, not one the old file supplies.
- `crates/dbfile/src/schema.rs`: the names are the caller's seeds. Every production caller passes a `static` slice: `SEEDS` in `pg-catalog` and in `pg-thumbcache`, through `Expected`, which takes `&'static` slices. The only other production path, `judge_conn`'s caller, passes `expected.seeds()`. `judge_conn` is `pub` and takes non-static strings, so a future caller could pass runtime data. Nothing in the type stops that.

### 6. Does a raw integer reaching a domain constructor matter?

Most proper: less than the probe made it sound. `Rating::new` and `TryFrom<u8>` already range-check. A raw `u8` that passes them is in range, whichever path it took.

The provenance wrapper matters where validity depends on the source, not on the range. Examples: XMP's `-1` means "rejected" and reads as no rating; an animation has at least one frame. For those cases the reader must return `Untrusted` and `checked` must be the only path to the domain value. Enforce it at the reader boundary, not with a lint.

`Rating` cannot be made file-only. The user's own ratings are built with it too (`crates/app/src/edit.rs`, `rate`).

The frame count needs its own domain type, since `Meta.frames` is raw `u32` today. Most proper: a newtype in `pg-model` in the style of `Rating`, `Orientation` and `RowHeight`, wrapping `NonZeroU32` so that zero is unrepresentable. The container already guarantees at least one.

### 7. How is the seal kept?

Most proper: tests in `pg-model`, not notes in a doc. Add `compile_fail` doctests for `Into`, `Deref`, `Display` and field access. Add a `disallowed-methods` entry for any `into_inner` someone adds later. The wrapper has no method that returns the value (see option 2 above).

### 8. What does the wrapper print, and what can it deserialize into?

Most proper: a hand-written `Debug` that prints `Untrusted(<unchecked>)`, and no derived `Debug`. A raw file value in a log is both a leak and noise. The validator's error is what belongs in a log.

No `Serialize` and no `Deserialize` on the wrapper. Deserializing into `Untrusted` is the bypass the doc warns about. Domain types may keep `try_from` serde, as `Rating` does.

### 9. Plan or prototype?

Most proper: a plan first, in `docs/plans/`, next to `DECODE_ONCE_PLAN.md` and `BOUNDED_MEMORY_PLAN.md`. The plan fixes the decisions above (crate, seal, sources, sinks), because they decide where the churn lands across `metadata`, `container`, `video`, `yandex` and `catalog`. Then implement the first slice: `tiff.uint`, `exif_date` and the frame count. Build once at the end.

### 10. Which verification tool?

Most proper now: the tools already in the repo. `fuzz/` exists, and the test suites use `proptest` (`crates/client/tests/tree_laws.rs`). Fuzz the readers, because the parsers are the untrusted surface.

`fuzz/` already covers part of the file readers. `metadata_read` runs `Meta::read` (EXIF, TIFF, XMP and the container) on arbitrary bytes, and `container_chunks` walks the PNG, WebP and GIF chunks. Not covered: the ffprobe parsing in `crates/video`, the Yandex JSON in `crates/yandex`, and `patch_orientation`, which writes to the file. Add targets for the ffprobe parsing and the Yandex JSON before the wrapper lands, so the refactor is checked against them.

Kani later, for the one function where truncation matters, such as `percent` or `xmp_value`. Kani needs a proof harness for each function, but no specifications. Prusti and Creusot need specifications written first. Kani is not installed.
