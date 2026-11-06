# Vendored crates

Crates we build from a fork of their upstream instead of crates.io. The forks live under `github.com/darktorres/`, each
holding the upstream's code with our local changes, and the root `Cargo.toml` pins every one by `rev` in
`[patch.crates-io]`. Nothing is copied into this directory any more (it holds only this README), so where a section says to copy into or
test from this directory, use a checkout of the fork instead. The sections below describe each local change, and each
fork's history shows it against the upstream.

Every Monday, `.github/workflows/fork-updates.yml` runs `tools/fork-updates.py`, which compares each fork's locked version
in `Cargo.lock` with the newest crates.io release of that crate. While one is newer, an open issue lists it.

## qt-build-utils

Copy of `qt-build-utils` 0.10.0 (cxx-qt's), so that the QML compiler (`qmlcachegen`) sees the `pg.gallery` module as
Qt's CMake build lays it out. Without that it compiles much of the QML to bytecode for the interpreter instead of C++,
and says so only with `--verbose`. The local changes are marked `photo-gallery`. They are in `register_qml_module` and
in `qt_metatypes`, the helper it calls (both in `src/lib.rs`), and in `QtToolQmlTypeRegistrar::compile`:

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

Fork of `honzasp/rust-turbojpeg` (`darktorres/rust-turbojpeg`, pinned at rev `945d853b`): `turbojpeg-sys` 1.2.0 with its
`libjpeg-turbo` sources, licensed `Unlicense OR MIT`. One local change, marked `photo-gallery` in `build.rs`: libjpeg-turbo
is built with `WITH_CRT_DLL=ON`, so it uses the dynamic C runtime as the rest of the app does. Without it, its objects
link the static runtime too and the linker warns `LNK4098` (two C runtimes in one binary).

To update: commit the new version in the fork, then change the `rev` in the root `Cargo.toml`, `eval/jpeg-decoders/Cargo.toml`
and `fuzz/Cargo.toml`. Keep the `WITH_CRT_DLL` line in `build.rs`.

## rusqlite

Fork of `rusqlite/rusqlite` (`darktorres/rusqlite`), pinned by rev in the root `Cargo.toml`: rusqlite 0.41.1 with one
local commit, marked `photo-gallery` in `src/cache.rs`: `StatementCache::get` sets a statement's cache key only when it
prepares the statement. A statement taken from the cache still carries the key it was cached under, yet `get` set a
new one on every call, copying the SQL into a new `Arc<str>` for each `prepare_cached` hit: one allocation per query,
hit or miss (docs/HEAP_PROFILE.md, site 11). A test there (`test_cache_hit_keeps_key`) checks that a hit keeps the key;
it fails without the change. Upstream still has it, in 0.41.1 and on master.

To update: rebase the commit onto the new release tag (`git rebase --onto <tag> v0.41.1`), change the `rev` and the
`version` requirement in the root `Cargo.toml`, then build and test. The fork's tests run from the fork:
`cargo test --features bundled`.

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
- `Encoder::rgb_to_thumb_hash_into` writes the hash into an array of `MAX_LEN` (25) bytes and returns its length,
  so the import's hash of each file is held inline and allocates nothing. The encoder writes every hash that way;
  the `Vec` functions copy it out at its length.

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
  weights. Upstream made a `Vec` per output pixel of each pass each call, 167 of them for a 100 × 67 fit (100 for the horizontal
  pass, 67 for the vertical).
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

Copy of `quick-xml` 0.42.0 (MIT), for the XMP packets the import reads (`pg_metadata`). The local changes,
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

## image-webp

Copy of `image-webp` 0.2.4 (MIT OR Apache-2.0). The local changes, marked `photo-gallery`, let a decoder keep what it
allocates from one image to the next; every pixel, every frame and every error is upstream's:

- `Scratch` (`src/decoder.rs`) holds the chunk map, the VP8 decoder's buffers (`Vp8Scratch`, `src/vp8.rs`: the
  three planes, the macroblocks, the top macroblocks and the borders, the partitions' buffers, the token
  probabilities), the lossless decoder's (`LosslessScratch`, `src/lossless.rs`: Huffman trees and their tables
  (`HuffmanPools`, `src/huffman.rs`), the code lengths, the entropy image, the transforms' data, the colour caches,
  a row of packed palette indices), the alpha plane and the RGBA it's decoded in, an animation's frame and its
  canvas. `WebPDecoder::new_with_scratch` decodes in them and gives them back with the error if the headers can't
  be read; `WebPDecoder::into_scratch` gives them back; `Scratch::trim` frees those over a size. Upstream made all of
  them anew for each image, the VP8 decoder its left borders anew for every macroblock row.
- The buffers are emptied or refilled before use, as `vec!` would make them, so they read as new ones; a decoder
  gives them back on its errors too.
- The VP8 decoder reads the last partition straight into its kept buffer, as `read_to_end` reads, instead of into a
  new `Vec` that was then copied into another (`init_partitions`). The partition sizes, the code length code
  lengths, the palette and its expanded lookup table are on the stack instead of in `Vec`s.
- `ArithmeticDecoder` holds its data in a `Vec` (upstream: a boxed slice, made with `into_boxed_slice`, which
  reallocated it), so that the buffer can be kept.
- `Vp8Decoder::new` and `LosslessDecoder::new`, left without callers, are gone (`with_scratch` instead).

Its unit tests give the same results as the registry's copy (23 passed, 1 ignored). A test of its own
(`src/decoder.rs`, `photo_gallery`) decodes 297 files in one kept scratch, forwards then backwards, with the scratch
kept whole, trimmed to nothing and trimmed at 4 KB after each file, and checks each decodes exactly as in new buffers
(dimensions, the image, every frame and its duration, errors and panics). The files: lossy ones with one and eight
partitions and both loop filters, lossy with an alpha plane compressed and raw, lossless photos and palettes of 2,
4, 16 and 200 colours, animations, all made by ImageMagick (`tests/photo-gallery/make.sh` makes them); animations the
test builds from those stills with frames of every kind (VP8, VP8L, ALPH with VP8) at offsets, blended or not,
disposed or not, with and without alpha and a background colour; a raw alpha plane with each filter and invalid
headers; and damaged copies of each, cut short or with bytes changed. Its test no longer prints its file counts on every run; its asserts are unchanged. Its tests run from
this directory:
`cargo test --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## gif

Copy of `gif` 0.14.2 (MIT OR Apache-2.0). The local changes, marked `photo-gallery`, let a decoder keep what it
allocates from one image to the next and read a buffered source as it is; every frame, every pixel and every error is
upstream's:

- `Scratch` (`src/reader/mod.rs`) holds the LZW decoders (weezl's, tables included), one per minimum code size, the
  extension buffer, the global and local palettes, the indices a frame is decoded to before they become RGBA, a
  frame's buffer (`read_next_frame`), and the XMP and ICC vectors. `DecodeOptions::read_info_with` decodes in them and
  gives them back with the error if the headers can't be read; `Decoder::into_scratch` gives them back;
  `Scratch::trim` frees those over a size. Upstream made all of them anew for each image, and an LZW decoder anew
  whenever a frame's code size differed from the frame before's. The buffers are emptied before use, so they read as
  new ones, and a decoder of a code size is reset (weezl's `reset`, which gives a decoder as `new` makes it).
- `read_info_with` reads its source as it is, where `read_info` wraps it in a `BufReader` of 8 KB, allocated per
  decoder, through which the file was copied. `Decoder` and `DecoderIter` have a second type parameter, the buffered
  reader, `BufReader<R>` by default, so `Decoder<R>` is still upstream's type and `image` builds against it unchanged.
- `Decoded::GlobalPalette` carries a `Vec` (upstream: a boxed slice, made with `into_boxed_slice`, which would
  reallocate a kept buffer to its length). This changes `StreamingDecoder`'s public API, which nothing here uses.
  The converter holds the global palette as a `Vec`, empty for none (upstream: an `Option`), so that an empty one
  keeps its allocation.
- `DecodeOptions::skip_metadata` skips the XMP and ICC application extensions as unknown ones are skipped: neither
  is collected nor counted against the memory limit, and `xmp_metadata` and `icc_profile` are `None`. Off by default.
  The app's decoder sets it (`pg_codec`), as its metadata reader finds a file's XMP itself.

Its unit tests give the same results as the registry's copy (6 passed, and the same 6 doctests failing: the published
crate lacks the `tests/samples` images they read). A test of its own (`src/reader/mod.rs`, `photo_gallery`) writes 150
GIFs: global and local palettes, transparency, interlaced frames, frames at offsets and past the screen, every LZW code
size from 2 to 11, application extensions (loop count, XMP, ICC, unknown), comments, an unknown extension, GIF87a,
indices past the palette, and damaged copies of each, cut short or with bytes changed. It reads each one five ways
(RGBA into a buffer as the app does, `read_next_frame` as RGBA, as indices and as LZW data, and with every check on
and a small memory limit), logging every frame, buffer, palette, error and piece of metadata. Over the file as it is,
a new decoder reads as upstream's over its `BufReader`, exactly. In 5-byte pieces, it reads the same except for two
things, as upstream's decoder does when its input is cut there: how much of a buffer a failing read fills, and whether
a missing LZW end code is found when the end code is checked. A decoder in one kept scratch, forwards then backwards,
kept whole, trimmed to nothing and trimmed at 600 bytes after each file, reads exactly as a new one over the same
reader, whether over the file or in 5-byte pieces. Its test no longer prints its tallies of partial pixels and end codes on
every run; they were never asserted. Its tests run from this directory:
`cargo test --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## tiff

Copy of `tiff` 0.11.3 (MIT). The local changes, marked `photo-gallery`, are in what a decoder allocates for small
values and for each chunk; every value, pixel and error is upstream's:

- `Entry::offset_field_reader` (`src/decoder/ifd.rs`) reads a tag's inline value (4 or 8 bytes) through a cursor over
  the entry's own array, where upstream copied it into a `Vec` first: one allocation per value read, for most tags.
- The chunk offsets and byte counts of an image (`Image::chunk_offsets`, `chunk_bytes`, `src/decoder/image.rs`) hold a
  single value inline (`U64s`), where upstream made a `Vec` of one for the single strip of most images. They are read
  by `Value::u64s_in`, which gives what `into_u64_vec` gives, and the same errors.
- A plane layout's offsets (`PlaneOffsets`) are worked out from their count and stride, where upstream collected them
  into a `Vec` for every chunk read; they are the multiples of the stride, and the count is checked as before.
- `decoder::Scratch` holds what reading chunks allocated anew for each chunk: the LZW decoder (weezl's, reset with
  `reset`, which gives one as `build` makes it) and the Deflate one (`flate2::Decompress`, reset with a full reset,
  which is what `new` makes), their input buffers (upstream: a `BufReader` each, 32 KiB for Deflate, up to 32 KiB for
  LZW), Group 4's input buffer and line, a JPEG chunk's data and samples and zune-jpeg's `Scratch` (the vendored
  copy's), the row a chunk is read into before its extra samples are dropped, and the floating point predictor's copy
  of each row (upstream: one per row). `Decoder::new_with_scratch` reads in it, giving it back with the error if the
  header or first image can't be read, and `into_scratch` gives it back; `Scratch::trim` frees buffers over a size.
  A chunk's reader is an enum of upstream's readers (`ChunkReader`), where upstream boxed it: every `Read` method the
  readers implement goes to them, as through the box. The Deflate reader's `read` is flate2 1.1.10's (what upstream's
  `ZlibDecoder` ran), over the kept `Decompress`. A JPEG chunk's metadata segments are checked but not copied
  (`set_skip_metadata`). The features the app doesn't build, zstd and WebP, keep upstream's boxed readers. Left as
  they are: the fax crate's Group 4 decoder, made for each chunk, allocates its own line state (a few blocks per
  chunk).
- The scratch also holds what reading an image's tags allocated anew for each image: spare vectors for the
  directory's entries, the extra samples, the chunk offsets and byte counts, the JPEG tables (and their `Arc`, when no
  other image holds it), the list tag values are read into (`Entry::val_in`, given back once converted), the IFD
  offsets and the cycle check's maps (cleared). An image's vectors go back to the scratch once the next image is read,
  so a decoder that fails to read the next image stays on the one before, as upstream's does. The tags are converted
  by reference (`Value::u64s_in`, `u16s_into`, `u8s_into`, which give what `into_u64_vec`, `into_u16_vec` and
  `into_u8_vec` give), and `Decoder::find_tag_unsigned_vec_into` is `find_tag_unsigned_vec` into the caller's
  vector. The header's byte order is read into an array, as `read_to_end` read it into a `Vec`.
- `Directory` holds its entries in a vector sorted by tag, one per tag, the last given (upstream: a `BTreeMap`, whose
  nodes can't be kept): appended, then stably sorted and the last of each tag kept, so it holds and iterates what the
  map did.
- Its `Cargo.toml` patches in the vendored zune-jpeg and zune-core when its tests run from this directory (as png's
  does), for zune-jpeg's `Scratch`.

Its tests give the same results as the registry's copy (45 passed, and the same 106 failing: the published crate
lacks the `tests/images` they read). Four tests of its own (`src/decoder/image.rs`, `photo_gallery`): `u64s_in`,
`u16s_into` and `u8s_into` give what `into_u64_vec`, `into_u16_vec` and `into_u8_vec` give for every kind of value,
errors included, into kept vectors; the counted plane offsets are upstream's
collected ones, for strides up to `usize::MAX` and the past-the-end check; classic and BigTIFF files in one strip
and many, of 8-bit, 16-bit and float samples, uncompressed, LZW, Deflate and PackBits, decode to the pixels they were
made from; and decoders in one kept scratch read 50 files (`tests/photo_gallery`, made by ImageMagick: every
compression the crate decodes, including JPEG with tables and Group 4, in strips of one row and many, tiles, planes
and BigTIFF, with both predictors, and damaged copies) exactly as new decoders do, forwards and backwards, kept
whole, trimmed to nothing and to 600 bytes. Without weezl's reset, or with the kept extra samples not emptied, that
test fails. Its tests run from this directory:
`cargo test --no-fail-fast --target-dir ../../target/vendor-test`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## zune-core

Copy of `zune-core` 0.5.3 (MIT OR Apache-2.0 OR Zlib), the byte readers of zune-jpeg (the JPEGs libjpeg-turbo
rejects). The local changes, marked `photo-gallery`:

- `ZReader::peek_at` (`src/bytestream/reader.rs`) lends the bytes from the source when it holds them in memory
  (`ZByteReaderTrait::peek_buffered`: a buffered reader's buffer, which for `ZCursor` is the whole source), where
  upstream copied them into a buffer of its own that grew to the largest peek, on each reader: zune-jpeg peeks at
  every EXIF, XMP, ICC and IPTC segment. The bytes, and the position after, are those of upstream's copy, which would
  succeed whenever they are lent; otherwise, and past its 20 MiB check, it runs as upstream.
- A crate-level `#![macro_use]` is removed: rustc ignores it, with a warning that a patched crate shows in every
  build of the workspace.

Its tests give the same results as the registry's copy, with and without `std` (4 and 6 passed, and 7 doctests).
Two tests of its own (`src/bytestream/reader.rs`, `photo_gallery`) compare `peek_at` with upstream's code for every
start, offset and length around the ends of the source, the bytes or the error and the position after: over a
`ZCursor`, which lends every peek, and over a 4-byte `BufReader`, which lends some and copies the others. Its tests
run from this directory: `cargo test --target-dir ../../target/vendor-test`, and again with `--features std`.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again.

## zune-jpeg

Copy of `zune-jpeg` 0.5.15 (MIT OR Apache-2.0 OR Zlib), the decoder of the JPEGs libjpeg-turbo rejects
(`pg_codec`, which `image`'s JPEG decoder also wraps). The local changes, marked `photo-gallery`, let a decoder
keep what it allocates from one image to the next and leave the metadata it isn't asked for; every pixel and every
error is upstream's:

- `Scratch` (`src/decoder.rs`) holds the components and their rows (`src/components.rs`: the coefficients of an MCU
  row, the upsampling rows, the rows above and the first row), the coefficients of each component (an MCU row's in a
  baseline image, the whole image's in a progressive one) and the upsampler's row. `JpegDecoder::new_with_scratch`
  decodes in them and `into_scratch` gives them back, however the decode ended; `Scratch::trim` frees those over a
  size. Upstream made all of them anew for each decoder, and pg-core uses two per image (one reads the headers, the
  other decodes into the colour space chosen from them). The buffers are filled as upstream's new ones were
  (`vec![value; len]` became a clear and a resize in place), so they decode as new ones.
- `JpegDecoder::set_skip_metadata` reads the EXIF, XMP, extended XMP, ICC, IPTC, gain map and MPF segments as upstream
  does, with the same checks and errors, but doesn't copy them: their getters give `None`. Off by default. The app's
  decoder sets it, as its metadata reader reads the file itself.
- A crate-level `#![macro_use]` is removed, as in zune-core.
- `rustfmt.toml` turns formatting off. `cargo fmt --all` formats this crate (a path dependency), and upstream lays its
  sources out with zune-image's own `rustfmt.toml` of nightly-only options, which the package doesn't carry: rustfmt's
  defaults would rewrite every file, and the local changes could no longer be told from upstream's.

Its tests give the same results as the registry's copy (7 unit tests, and 10 integration tests with 1 ignored). A
test of its own (`src/photo_gallery.rs`) decodes 48 small JPEGs (`tests/photo_gallery`, made by libjpeg through
Pillow, ImageMagick and jpegtran): RGB at every subsampling, gray, CMYK and YCCK, baseline and progressive, restart
markers, arithmetic coding, EXIF, a two-segment ICC profile and XMP, sizes from 1×1 to 160×96, and copies cut short or
with bits changed. Each is decoded as pg-core decodes it, into four colour spaces, strict and not. Decoders in one kept
scratch, forwards then backwards, kept whole, trimmed to nothing and trimmed to 600 bytes after each file, give
exactly what a new decoder gives: dimensions, pixels, errors and metadata, and no metadata with the skip. A copy whose
row buffers kept their old contents fails it. A second checks that the components' vector stays in the scratch from
one decode to the next. Its tests run from this directory:
`cargo test --target-dir ../../target/vendor-test`.

Checked once against the registry's copy, linked side by side in a harness outside the repository: the 5 JPEGs
libjpeg-turbo rejects at any scale (1 in the library, and the test corpus's 4 cut short), 569 synthetic files (the
kinds above at sizes up to 1023×769, and 300 damaged copies) and 150 library JPEGs, through pg-core's front end, three
passes in shuffled orders through one kept scratch, against a new registry decoder per file. No difference in 2,172
decodes: dimensions, colour spaces, pixels, errors, and the metadata without the skip. Decoded by pg-core itself, the
fallback and synthetic files give the same pixels and errors as before the change.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the changes again and keep `rustfmt.toml` and
`tests/photo_gallery`.

## dhat

dhat-rs 0.3.3 (`dhat`, MIT OR Apache-2.0), patched in the `darktorres/dhat-rs` fork (pinned in `crates/gallery/Cargo.toml`; before that, `patches/dhat` in
phpstan-rs, commit `13675fd3`) so that heap
profiles with whole backtraces (`trim_backtraces(None)`) take minutes rather than hours. Upstream unwinds while it
holds its spinlock, so every thread queues behind one unwind, and each new global peak sweeps every program point. The
patch takes backtraces before the lock, keeps them as return addresses resolved once at exit, records the peak lazily
and locks a `std::sync::Mutex`. Its README lists the changes; the output (`dhat-heap.json`) is upstream's.

It also walks frame pointers instead of unwind tables, but only on Linux and macOS (`build.rs`). On Windows it
unwinds with `backtrace` (`RtlVirtualUnwind`), still outside the lock. Only `gallery`'s `dhat-heap` feature uses it
(docs/PROFILING.md "Tool: dhat"). It is a git dependency of `gallery` (`crates/gallery/Cargo.toml`), pinned by rev to
`darktorres/dhat-rs`, and it keeps its own `rustfmt.toml`, as upstream's.

To update: commit the change in the fork, then change the `rev` in `crates/gallery/Cargo.toml`.
