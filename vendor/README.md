# Vendored crates

Crates we build from a local copy instead of crates.io. The root `Cargo.toml` routes them in with `[patch.crates-io]`
and leaves this directory out of the workspace (`exclude`), so its lints, formatting and tests are the upstream's.

## qt-build-utils

Copy of `qt-build-utils` 0.10.0 (cxx-qt's), so that the QML compiler (`qmlcachegen`) sees the `pg.gallery` module as
Qt's CMake build lays it out. Without that it compiles much of the QML to bytecode for the interpreter instead of C++,
and says so only with `--verbose`. The local changes are marked `photo-gallery` and are all in `register_qml_module`
(`src/lib.rs`) and `QtToolQmlTypeRegistrar::compile`:

- `qmltyperegistrar` runs before `qmlcachegen`, not after. qmlcachegen reads the module's `plugin.qmltypes`, and so
  it saw the previous build's types, or none in a clean build.
- `qmltyperegistrar` gets Qt's metatypes (`--foreign-types`, from `QT_INSTALL_ARCHDATA/metatypes`). Without them, it
  declares each Qt type that the module's types reference (`QString`, `QUrl`, `QStringList`, `QVariant`) as an opaque
  type of the module. That hides the real one: a `QString` property has no `length`, and a `QUrl` can't be assigned.
- The QML files are copied beside the module's `qmldir`, which lists them by paths relative to itself. qmlcachegen
  finds the module's own components (`FilterBar` in `Main.qml`) through it.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the three changes again, or drop the copy once upstream does
them.

## turbojpeg-sys

Copy of `turbojpeg-sys` 1.2.0 (with its `libjpeg-turbo` sources). One local change, marked in `build.rs`: libjpeg-turbo
is built with `WITH_CRT_DLL=ON`, so it uses the dynamic C runtime as the rest of the app does. Without it, its objects
link the static runtime too and the linker warns `LNK4098` (two C runtimes in one binary).

To update: copy the new version from the cargo registry over this directory, then add the `WITH_CRT_DLL` line back
to `build_vendor` in `build.rs`.

## rusqlite

Copy of `rusqlite` 0.37.0. One local change, marked `photo-gallery` in `src/cache.rs`: `StatementCache::get` sets a
statement's cache key only when it prepares the statement. A statement taken from the cache still carries the key it
was cached under, yet `get` set a new one on every call, copying the SQL into a new `Arc<str>` for each
`prepare_cached` hit: one allocation per query, hit or miss (docs/HEAP_PROFILE.md, site 11). A test there
(`test_cache_hit_keeps_key`) checks that a hit keeps the key; it fails without the change. Upstream still has it, in
0.41.1 and on master.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the change again, or drop the copy once upstream does it. Its
tests run from this directory: `cargo test --features bundled --target-dir ../../target/vendor-test`.

## thumbhash

Copy of `thumbhash` 0.1.0 (MIT). The local changes, marked `photo-gallery` in `src/lib.rs`, are in how it holds its
working data; every hash and every decoded image is upstream's, bit for bit:

- `Encoder` keeps the encoder's four planes (luminance, the two colour differences and alpha, at most 100 × 100 `f32`
  each) from one image to the next, each sized for the image before it is filled rather than grown by `push`.
  `rgba_to_thumb_hash` is upstream's function on a new `Encoder`.
- `Encoder::rgb_to_thumb_hash` reads RGB as it is, alpha 1.0. That is what upstream computes for RGBA with every alpha
  255 (`255 as f32 / 255.0` is 1.0 exactly), so the import no longer copies each image into RGBA for it.
- A channel's AC terms (at most 49: both sides are at most 7) and the cosine rows are arrays on the stack, in the
  encoder and the decoder. Upstream allocated a `Vec` for each, 9 per encode and 6 per decode. Decoding now
  allocates only the image it returns.

Checked against the registry's 0.1.0, once, with a throwaway harness: 20,000 random images (1–100 px a side, opaque
and with alpha, noise and gradients) hash identically through the free function, a reused `Encoder` and, opaque, its
RGB entry point; their hashes and 200,000 random byte strings of 0–40 bytes decode identically, errors included. A
test in `src/lib.rs` keeps the RGB and reuse cases.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok` and
`.cargo_vcs_info.json`), then make the changes again.

## fast_image_resize

Copy of `fast_image_resize` 6.1.0 (MIT OR Apache-2.0). The local changes, marked `photo-gallery`, are in how the
convolution holds its coefficients; the arithmetic, and so every resized pixel, is upstream's:

- `Resizer` keeps the last coefficients of each axis (`AxisCoefficients` in `src/resizer.rs`), with what they were
  computed for: the source size, crop, output size, filter and kernel mode. A resize that matches on an axis reuses
  them, as runs of same-size camera files do; any other computes them again into the same buffers. Upstream computed
  them per call into new vectors.
- `Coefficients` (`src/convolution/mod.rs`) keeps its normalized forms beside the weights, made when first asked for:
  i16 for 8-bit images (`Normalizer16`) and i32 for 16-bit ones (`Normalizer32`), each one buffer laid out like the
  weights. Upstream made a `Vec` per output pixel each call, 169 of them for a 100 × 67 thumbnail-hash fit.
- The chunks a convolution reads (`Chunks`, `Chunk`) are views over those buffers. `get_chunks` no longer collects a
  `Vec`. The second pass of a two-pass resize reads its bounds through a `shift`, where upstream rewrote them.
- The `Convolution` trait borrows the coefficients mutably instead of taking them by value. The SIMD backends changed
  only in the types they pass chunks by. The NEON (aarch64) and WebAssembly backends compile (`cargo check --target
  aarch64-pc-windows-msvc`, `wasm32-unknown-unknown`) but don't run here.

Checked, once, against the registry's 6.1.0 with a throwaway harness: 18,000 resizes (6,000 random cases on each of
the native, SSE4.1 and AVX2 paths) over every pixel type, filter and algorithm, with random sizes up to 900 px, crops
and alpha. One `Resizer` was reused throughout, a third of the cases repeating the last sizes, so the kept coefficients
were hit, missed and shared across pixel types and pass orders. Every output was identical byte for byte, with
debug assertions on. With the shift's reset removed, the harness failed at case 58. The crate's own tests give the
same results as the registry copy's, including the same 14 checksum mismatches, which upstream's copy also has
on this machine. Their images aren't in the published crate (`data/` in the repository at tag `v6.1.0`). A test in
`src/resizer.rs` keeps the reuse case.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## quick-xml

Copy of `quick-xml` 0.42.0 (MIT), for the XMP packets the import reads (`pg_core::metadata`). The local changes,
marked `photo-gallery`, are in what a reader allocates; every event it gives, and every error, is upstream's:

- `NsReader::from_str_with(text, ReaderBuffers)` reads as `NsReader::from_str` does, in the buffers an earlier
  reader gave back (`NsReader::into_buffers`): the open elements' names (`src/reader/state.rs`) and the namespace
  bindings in scope (`src/name.rs`). A worker reading packet after packet allocates only when one needs more than
  any before it; upstream made both anew for each reader.
- The duplicate-attribute check (`src/events/attributes.rs`, `Keys`) keeps the first 32 names of a tag inline, the
  rest in a `Vec` as before, and so allocates nothing for a tag with at most 32 attributes. Upstream pushed every
  name into a `Vec`: one allocation per tag with attributes, more as it grew.

Its unit and doc tests give the same results as the registry's copy (245 and 62 passed, 4 and 2 ignored). Two tests
of its own (`src/reader/ns_reader.rs`, `photo_gallery`): readers in kept buffers read as new ones, document after
document, malformed ones included; and duplicates are found on both sides of the inline names, at the positions
upstream reports. Its tests run from this directory: `cargo test --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## png

Copy of `png` 0.18.1 (MIT OR Apache-2.0). The local changes, marked `photo-gallery`, let a decoder keep what it
allocates from one image to the next; every pixel, every `Info` and every error is upstream's:

- `Scratch` (`src/decoder/mod.rs`) holds the chunk buffer, the inflater (its `fdeflate` decompressor, tables
  included), the buffers rows are unfiltered and returned in, and the vectors of the chunks `Info` keeps a copy of
  (palette, transparency, significant bits, background, EXIF, ICC profile). `Decoder::new_with_scratch` decodes in
  them, `Reader::into_scratch` gives them back, and `Decoder::into_scratch` and `Decoder::read_info_or_scratch` do so
  when no reader is made or `read_info` fails. `Scratch::trim` frees those over a size. Upstream made all of them
  anew for each image, the inflater's 18 KB of tables among them.
- The buffers are emptied before use, so they read as new ones. The chunk buffer's growth is counted from upstream's
  128 bytes (`ChunkState::capacity`), not from the kept buffer's capacity: how much each call reads, and what is
  reserved from the limits, are upstream's, so the same files exceed the same limits.
- `ZlibStream::reset` resets the decompressor in place (`fdeflate`'s `Decompressor::reset`, below) instead of making
  a new one, which an APNG did for every frame.
- Its own `Cargo.toml` patches `fdeflate` to `../fdeflate` for its own build and tests; as a dependency, the app gets
  it through the root's patch.

Its unit tests give the same results as the registry's copy (73 passed, 1 ignored, and the same 8 failures: the
published crate lacks the `tests/` images they read). Two tests of its own (`src/decoder/mod.rs`, `photo_gallery`):
PNGs of every colour type and depth, interlaced or not, with every filter, palettes, transparency and the other kept
chunks, an APNG, and damaged copies, decode in one kept scratch exactly as in new buffers (pixels, `Info`, errors),
with every transformation and under limits; and the limits count a kept chunk buffer as a new one. Its tests run from
this directory: `cargo test --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## fdeflate

Copy of `fdeflate` 0.3.7 (MIT OR Apache-2.0). One local change, marked `photo-gallery` in `src/decompress.rs`:
`Decompressor::reset` puts a decompressor back in the state `Decompressor::new` gives (the primary Huffman tables
zeroed, the secondary ones emptied, every other field as `new` sets it), keeping the tables' allocations, so that png
can reuse one decompressor for every image. A test of its own (`photo_gallery`) checks that a reset decompressor is in
a new one's state, field by field, and decodes streams of every kind, damaged and cut short ones included, exactly as
a new one, whatever it decoded before. Its tests give the registry copy's results plus that one (15 passed). They run
from this directory: `cargo test --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the change again.

## dhat

dhat-rs 0.3.3 (`dhat`, MIT OR Apache-2.0), patched in phpstan-rs (`patches/dhat`, commit `13675fd3`) so that heap
profiles with whole backtraces (`trim_backtraces(None)`) take minutes rather than hours. Upstream unwinds while it
holds its spinlock, so every thread queues behind one unwind, and each new global peak sweeps every program point. The
patch takes backtraces before the lock, keeps them as return addresses resolved once at exit, records the peak lazily
and locks a `std::sync::Mutex`. Its README lists the changes; the output (`dhat-heap.json`) is upstream's.

It also walks frame pointers instead of unwind tables, but only on Linux and macOS (`build.rs`). On Windows it
unwinds with `backtrace` (`RtlVirtualUnwind`), still outside the lock. Only `gallery`'s `dhat-heap` feature uses it
(docs/PROFILING.md "Tool: dhat"). It keeps its own `Cargo.lock`, and its dependencies are pinned exactly, at the
versions the workspace's `Cargo.lock` has. Being a path dependency, it is formatted by `cargo fmt --all`: its
`rustfmt.toml` keeps it at rustfmt's defaults, as upstream's.

To update: copy phpstan-rs's `patches/dhat` over this directory (without `target/`).
