# Heap profile: `gallery scan` and `gallery import` (dhat, 2026-11-01)

Every Rust heap allocation of a scan and an import, charged to the code of ours that decides to make it (the
innermost frame in `pg_core`, `pg_ingest` or `gallery` that isn't a container or conversion helper), and each site
read at its source lines. How the profiles are made and read: PROFILING.md "Tool: dhat".

**Profiles:** `target/dhat/gallery.exe` (commit d2205ba, `--profile dhat --features dhat-heap`), untrimmed
(`PG_DHAT_UNTRIMMED=1`), in `D:\PhotoGallery\scratch_dhat`:

- `scan-untrimmed.json`: `gallery scan D:\PhotoGallery\borders_sample\_preview` into a new catalog.
- `import-w1-untrimmed.json`: `gallery import --workers 1` after that scan. One worker gives the same counts on
  every run; `import-untrimmed.json` is the same import with the default 12 workers, for the peak only.

**The workload is small, and it is what the numbers describe.** 305 JPEGs of 5.3 MB in all (about 17 KB and
~87 k pixels each), none with EXIF or XMP, in a catalog with no earlier analyses. So the profiles don't show: the
tier-1 index rebuilt from existing hashes, files bigger than the decode's 512 × 320 floor, DCT-scaled decodes,
non-JPEG decodes (full size, `decode::decode_other`), EXIF that parses, videos. Where a finding depends on scale, it
says what it would be at scale, worked out from the code; those are estimates, not measurements.

## Totals

| | Import, 1 worker | Import, 12 workers | Scan |
|---|---:|---:|---:|
| Allocated | 425.6 MB in 167,437 blocks | 425.6 MB in 167,454 blocks | 544 KB in 2,472 blocks |
| Per file | 1.40 MB in 549 blocks | | 1.8 KB in 8 blocks |
| Global peak | 27.6 MB in 1,041 blocks, at 0.54 s of 1.05 s | 33.2 MB in 1,380 blocks, at 0.70 s of 1.35 s | 101 KB in 318 blocks |
| At exit | 1,076 B in 5 blocks | 1,076 B in 5 blocks | 1,028 B in 4 blocks |

By what the import does, of the one-worker totals:

| Work | Bytes | Blocks |
|---|---:|---:|
| PDQ: `dct16`'s two planes, `box_columns`' row | 211.9 MB (49.8%) | 1,220 (0.7%) |
| JPEG decode (`decode_jpeg`'s pixels) | 79.2 MB (18.6%) | 305 (0.2%) |
| Resizing (`resize::resize`, inside fast_image_resize) | 41.7 MB (9.8%) | 53,367 (31.9%) |
| ThumbHash (our RGBA copy and the crate's planes) | 41.2 MB (9.7%) | 4,575 (2.7%) |
| Tier-1 index (`PdqIndex`) | 25.3 MB (5.9%) | 4,890 (2.9%) |
| EXIF, parsed twice (kamadak-exif's container reader) | 19.9 MB (4.7%) | 95,692 (57.2%) |
| File reads (`std::fs::read`, its UTF-16 path, `long_path`) | 4.9 MB (1.2%) | 915 (0.5%) |
| Metadata segment walks (`jpeg_segments`) | 0.4 MB | 1,830 (1.1%) |
| The catalog's statements (`store_analysis`, `sync_file_rating`) | 0.4 MB | 3,059 (1.8%) |
| The work list, the batch, the pipeline, the command line | 0.6 MB | 1,584 (0.9%) |

## The peak

**Import, one worker: 27.6 MB.** 91% of it is the tier-1 index's empty tables.

| Live at the peak | Bytes | Blocks | Why |
|---|---:|---:|---|
| `PdqIndex::default` (`pdq_index.rs:30`) | 25,165,824 (91.1%) | 16 | 16 chunk tables × 65,536 buckets × 24 B, each an empty `Vec<u32>` header, made before any hash is inserted |
| `dct16`'s luma and temp planes (`pdq.rs:113`, `:117`) | 1,036,800 (3.8%) | 2 | The file being hashed: two `width × height` f32 planes |
| File buffers (`pipeline.rs:72`) | 700,710 (2.5%) | 41 | 32 in the read channel, 8 held by blocked readers, 1 being analyzed |
| The decoded image (`decode.rs:177`, reached at `decode.rs:141`) | 388,800 (1.4%) | 1 | The file being analyzed, RGB |
| The batch (`import.rs:41`) | 232,000 (0.8%) | 1 | `Vec::with_capacity(500)` of `(usize, Result<Analysis>)`, 464 B each |
| The work list (`catalog.rs:717`) | 46,972 | 306 | `WorkItem`s and their paths |
| The paths' copy (`import.rs:39`) | 25,620 | 306 | `pipeline::run` takes `&[String]`, so every path is cloned |
| Analyses waiting for the batch's commit | 4,727 | 326 | 163 ThumbHashes (25 B) and format names (4 B): the batch commits at 500, so all of this import's wait for the final flush |
| The rest | 8,632 | 42 | channels, threads, `box_columns`' row, clap's defaults |

**Import, 12 workers: 33.2 MB.** The same index (75.7%), plus up to 12 files in flight: 11 decoded images (3.4 MB),
6 dct16 planes (2.4 MB), 52 file buffers (0.9 MB), resizer buffers (0.7 MB), ThumbHash planes (0.3 MB), the batch
(0.2 MB). 52 buffers is the pipeline's bound with 8 readers and 12 workers: the channel's `readers * 4` = 32 slots,
8 readers each holding a file it can't send yet, 12 workers each holding the file it analyzes.

**Scan: 101 KB,** at 14 ms of 32 ms: `Dir::entries`' 64 KB listing buffer (`scan.rs:84`), its `Vec<DirEntry>` at
28.7 KB (`scan.rs:116`) and the 305 names (`scan.rs:114`, 6.1 KB).

## At exit

Nothing leaks. What is live when the profile is written is held by design:

| Bytes | Site | What |
|---:|---|---|
| 1,024 | `cli.rs:181` (import), `cli.rs:163` (scan) → `println!` | std's stdout `LineWriter` buffer, a process-lifetime `OnceLock` |
| 48 | `pipeline.rs:107` → crossbeam-channel | The main thread's thread-local waiting `Context`, freed when the thread ends (after the profile is written) |
| 2, 1, 1 | `cli.rs:108`, `cli.rs:33`, `cli.rs:111` | clap derive's `default_value_t` strings (`dups --distance`'s, `--readers`' `"8"`, `dups --show`'s `"0"`), kept in statics to be `&'static str` |

## Every site of the import (one worker)

Sorted by blocks. "Peak" is what was live at the global peak. clap's 30 sites of `Command` building are one row at
the end.

| # | Blocks | Bytes | Peak | Site | What, and what it would take |
|---:|---:|---:|---:|---|---|
| 1 | 53,062 | 35.6 MB | 0 | `resize.rs:63` `Resizer::new().resize` | Inside fast_image_resize, per call (one call per file here, the ThumbHash's 100 px fit; files bigger than 512 px add the PDQ fit): the temp image of the two-pass convolution (305 × 72 KB = 22.0 MB, `get_temp_image_from_buffer`), the coefficients (`precompute_coefficients`, 4 blocks, 9.8 MB) and a `Vec<i16>` per output column and row with a list of them per pass (`Normalizer16::new`, 169 blocks for a 100 × 67 output). A `Resizer` per worker would keep its temp buffer across calls (305 of the blocks, 22 MB); the coefficient vectors are the crate's, per call. |
| 2 | 47,846 | 10.0 MB | 0 | `metadata.rs:47` `exif_rating` → `read_from_container` | See "EXIF" below: kamadak-exif walks the whole file when there is no EXIF, a `Vec` per 0xFF byte. |
| 3 | 47,846 | 10.0 MB | 0 | `decode.rs:221` `exif_orientation` → `read_from_container` | The same parse of the same bytes, a second time. |
| 4 | 4,849 | 77.6 KB | 0 | `pdq_index.rs:54` `tables[c][chunk].push(pos)` | A new bucket's first block (4 × u32), 16 per inserted hash. See "The tier-1 index". |
| 5 | 4,270 | 33.0 MB | 4.1 KB | `analyze.rs:127` `thumbhash::rgba_to_thumb_hash` | The crate's four f32 planes (`lib.rs:36`–`39`, 4 × 26.8 KB per file), its AC coefficient vectors (`lib.rs:56`, `:58`, 9 per file) and the 25-byte hash (`lib.rs:109`), which the analysis keeps. The crate's API; nothing to change on our side. |
| 6 | 1,830 | 351 KB | 0 | `metadata.rs:277` `jpeg_segments`' `out.push` | The segment list growing (2 blocks per walk), walked three times per file: `read_rating` → `jpeg_xmp`, `read_taken` → `jpeg_xmp`, `read_taken` → `exif_block`. One walk per file would serve all three. |
| 7 | 915 | 170 KB | 0 | `catalog.rs:766` `format!("UPDATE … {HAS_BORDERS} …")` | A constant string built per file (2 blocks), plus rusqlite's cache key for it (below). `HAS_BORDERS` as a macro, with `concat!`, would make it a `&'static str`. |
| 8 | 618 | 76.9 KB | 47.0 KB | `catalog.rs:717` `pending_import`'s rows | The work list: each `WorkItem`'s path (304 + 1), the kind read as a `String` and dropped (`r.get::<_, String>(5)`, 304 + 1, 5 B each; `get_ref(5)?.as_str()` reads it in place), and the `Vec` growing (8). |
| 9 | 610 | 4.9 MB | 701 KB | `pipeline.rs:72` `std::fs::read(long_path(path))` | The file's bytes (305), and std's UTF-16 copy of the path (305, `to_u16s`, std's). See "The read pipeline". |
| 10 | 610 | 599 KB | 1.4 KB | `pdq.rs:182` `box_columns`' `sum` row | One `width` f32 row per call, two calls per file. |
| 11 | 308 | 19.7 KB | 0 | `catalog.rs:735` `prepare_cached("DELETE FROM pdq_pairs …")` | rusqlite copies the SQL into a new `Arc<str>` as its cache key on every call, hit or miss (`cache.rs:148`, `set_statement_cache_key`); the 3 others are the cache's first insert. |
| 12 | 307 | 19.7 KB | 0 | `catalog.rs:768` `prepare_cached("DELETE FROM border_sides …")` | The same, for this statement. |
| 13 | 305 | 105.7 MB | 518 KB | `pdq.rs:117` `dct16`'s `temp` plane | `vec![0.0f32; width × height]`, per file. With `luma` it is half of all bytes. Two planes per worker, kept across files, would remove both. |
| 14 | 305 | 105.7 MB | 518 KB | `pdq.rs:113` `dct16`'s `luma` plane | The same size, collected from the RGB. |
| 15 | 305 | 79.2 MB | 389 KB | `decode.rs:141` → `decode_jpeg`'s `vec![0u8; w × h × 3]` (`decode.rs:177`) | The decoded image, per file. It becomes `Rgb`, which the analysis reads from three places; it is the output, not a scratch buffer. |
| 16 | 305 | 8.2 MB | 0 | `analyze.rs:126` the RGBA copy for ThumbHash | The crate wants RGBA; the image is RGB. |
| 17 | 305 | 6.1 MB | 0 | `resize.rs:61` `Image::new` | The resized output, which becomes the returned `Rgb`. |
| 18 | 305 | 154 KB | 0 | `catalog.rs:736` `prepare_cached("INSERT OR REPLACE INTO analysis …")` | rusqlite's cache key, 504 B (the SQL's length) per file. |
| 19 | 305 | 29.3 KB | 0 | `catalog.rs:910` `prepare_cached("SELECT EXISTS … file_writes …")` | The same. |
| 20 | 305 | 22.0 KB | 0 | `catalog.rs:906` `prepare_cached("SELECT value FROM file_ratings …")` | The same. |
| 21 | 305 | 18.3 KB | 18.3 KB | `import.rs:39` `i.path.clone()` | A second copy of every path, live for the whole import. At 190k pending files, about 190k blocks and 15 MB. `pipeline::run` could take a `Fn(usize) -> &str`. |
| 22 | 305 | 18.3 KB | 0 | `pipeline.rs:39` `long_path`'s `path.to_string()` | A copy of the path for every read, though the prefix is needed only past 240 characters. A `Cow<str>` would borrow it. |
| 23 | 305 | 9.8 KB | 0 | `catalog.rs:754` `p.pdq.hash.0.to_vec()` | The PDQ hash copied into a `Vec` to bind it; `&p.pdq.hash.0[..]` binds the slice. |
| 24 | 305 | 4.9 KB | 0 | `catalog.rs:749` `p.pixel_hash.to_vec()` | The same, for the pixel hash. |
| 25 | 305 | 1.2 KB | 652 B | `analyze.rs:76` `decoded.format.name().to_string()` | A static name copied into `Analysis.format: String`; a `&'static str` field needs no block. |
| 26 | 88 | 9.3 KB | 36 B | `cli.rs:133` `Cli::parse_from` | clap parsing the arguments. Once per run. |
| 27 | 25 | 1.5 KB | 1.3 KB | `pipeline.rs:67` `s.spawn` (readers) | 8 threads: their `Thread`, packet and name. Once per pass. |
| 28 | 16 | 25.2 MB | 25.2 MB | `pdq_index.rs:30` `vec![Vec::new(); 1 << 16]` | The 16 tables. See "The tier-1 index". |
| 29 | 15 | 902 B | 0 | `main.rs:49` `std::env::args_os().collect()` | The arguments. Once. |
| 30 | 10 | 672 B | 576 B | `pipeline.rs:73` `raw_tx.send` | crossbeam's per-thread waiting context and waker entries (readers). |
| 31 | 8 | 32.6 KB | 0 | `pdq_index.rs:57` `hashes.push` | Growing by doubling. `reserve` before a known number of inserts (the rebuild) would make it one block. |
| 32 | 8 | 8.2 KB | 0 | `pdq_index.rs:56` `ids.push` | The same. |
| 33 | 8 | 4.1 KB | 0 | `pdq_index.rs:58` `seen.push` | The same. |
| 34 | 6 | 109 KB | 0 | `catalog.rs:46` `baseline()` | The schema text, three `replace`s (2 blocks each), on every open, though a current catalog doesn't run it. Once per run. |
| 35 | 3 | 812 B | 0 | `catalog.rs:502` `pending_import_sql`'s `format!` | Once per import. |
| 36 | 3 | 796 B | 0 | `catalog.rs:510` the same | Once per import. |
| 37 | 3 | 168 B | 144 B | `pipeline.rs:84` `s.spawn` (the worker) | One thread. |
| 38 | 2 | 2.6 KB | 2.6 KB | `pipeline.rs:61` `bounded(opts.workers * 4)` | The results channel. |
| 39 | 2 | 1.9 KB | 1.9 KB | `pipeline.rs:60` `bounded(opts.readers * 4)` | The read channel. |
| 40 | 2 | 148 B | 0 | `cli.rs:140` `create_dir_all` | std's path conversion. |
| 41 | 2 | 144 B | 144 B | `pipeline.rs:107` `for … in res_rx` | crossbeam's context for the main thread (48 B of it is the exit's). |
| 42 | 2 | 108 B | 72 B | `cli.rs:141` `catalog_dir.join(…)` | The catalog's path. |
| 43 | 2 | 84 B | 32 B | `catalog.rs:333` `Connection::open` | rusqlite's path and handle. |
| 44 | 1 | 232 KB | 232 KB | `import.rs:41` `Vec::with_capacity(BATCH)` | The batch, for 500 analyses. |
| 45 | 1 | 7.3 KB | 7.3 KB | `import.rs:39` `collect()` | The `Vec` of the paths' copies (row 21). |
| 46 | 1 | 1.0 KB | 0 | `cli.rs:181` `println!` | std's stdout buffer, held to the end. |
| 47 | 1 | 384 B | 384 B | `pdq_index.rs:30` `.collect()` | The outer `Vec` of the 16 tables. |
| 48 | 1 | 110 B | 0 | `control.rs` `Progress::line` (from `Control::finish`) | The last progress line. |
| 49 | 4 | 352 B | 0 | `catalog.rs:764`, `:767`, `:908`, `:911` | rusqlite's statement cache: the first insert of four statements. |
| 50 | 1 | 40 B | 40 B | `pipeline.rs:63` `std::thread::scope` | The scope's shared state. |
| 51 | 3 | 4 B | 4 B | `cli.rs:108`, `:33`, `:111` | clap's default-value strings (the exit's). |
| 52 | 274 | 107 KB | 0 | `cli.rs:20`–`112` (30 sites: `augment_args`, `augment_subcommands`) | clap derive building the `Command` from the attributes in `cli.rs`: each argument's name, help, value parser and default (`Arg::help`, `value_name`, `default_value`, `conflicts_with`, `Command::arg`, `subcommand`). Read at each attribute line; all of it is startup, once per run. |

### EXIF

`exif_orientation` (`decode.rs:221`) and `exif_rating` (`metadata.rs:135`) each call kamadak-exif's
`Reader::read_from_container` on the file's bytes: 157 blocks per call, 57% of the import's blocks. For a JPEG, it
reads the first 4 KB into a `Vec` that grows by doubling (`reader.rs:141`, 9 blocks), then
`jpeg::get_exif_attr` (`jpeg.rs:62`–`99`) loops over the segments: each segment's body goes into a new
`vec![0; len]`, and it returns only at an APP1 `Exif` segment or at EOI. It doesn't stop at SOS. Without EXIF it
goes on through the entropy-coded data, and each `read_until(0xFF, &mut Vec::new())` puts the bytes between two
0xFF bytes in a new `Vec`, which it throws away. The 305 files have 8 segments before SOS, then SOS's, and 129.9
0xFF bytes in their scan data on average: 9 + 9 + about 130 = 148 of the 157 blocks measured. Both calls go through
the whole file.

A camera JPEG has its EXIF in one of its first segments, and the parse returns there. Screenshots, downloads, edited exports and
files stripped of metadata don't, and for those the cost grows with the file: at these files' rate (a 0xFF byte
per 118 bytes of scan data), a 5 MB JPEG has about 44,000, so about 88,000 blocks a file, and two passes over it.

**Lead:** find the APP1 `Exif` segment with our own `jpeg_segments` (it stops at SOS), as `read_taken` already does
(`exif_block` → `Reader::read_raw`), and parse it once per file for the orientation, the rating and the time taken.
The three `jpeg_segments` walks (row 6) and the second parse go with it.

### The tier-1 index

`PdqIndex::default` (`pdq_index.rs:25`–`34`) makes 16 tables of 65,536 buckets each, every bucket an empty
`Vec<u32>`: 24 B × 1,048,576 = 25.2 MB, before any hash. It is 91% of this import's peak, and it is made by every
import with files to analyze. Each insert then pushes into one bucket of each table, and a bucket's first push is a
heap block (row 4). The scratch catalog was empty, so the rebuild from existing hashes (`import.rs:35`–`37`, after
`pdq_hashes()` collects them) wasn't in the profile. For the pinterest catalog (190k files), it would be about 190k ×
16 = 3.0 M positions in about a million non-empty buckets: on the order of a million blocks with their growth and
the heap's per-block overhead, on top of the 25 MB, and the `(i64, Hash)` list of 190k × 40 B = 7.6 MB while it is
built.

**Lead:** a flat layout per chunk (bucket offsets and one `Vec<u32>` of positions, built in two passes from the
rebuilt hashes), with the inserts made during the import kept in a small overflow map. Worth a profile of an import
into a copy of the pinterest catalog first, to measure it rather than estimate it.

### The read pipeline

Each reader holds a whole file (`std::fs::read`). The read channel holds `readers * 4` of them (`pipeline.rs:60`),
on top of the one each blocked reader holds and the one each worker analyzes: 8 × 4 + 8 + 12 = 52 files with the
defaults on this machine's 12 logical CPUs, the 52 measured at the 12-worker peak. Here that is 0.9 MB. With camera
JPEGs of 5–10 MB it would be 260–520 MB, far above everything else, and it would be the import's peak. FINDINGS
"I/O" asks for 8 reads in flight to reach the HDD's sequential speed; the four-deep queue per reader on top of them
is what to question, or a bound in bytes rather than files.

### The per-file scratch buffers

Per file, the import allocates and frees: `dct16`'s two planes (2 × 4 B × pixels), the decoded image, the resizer's
temp image and coefficients, ThumbHash's RGBA copy and the crate's four planes. That is 1.4 MB and 549 blocks per
file here. At real sizes the decode's floor (512 × 320) and the PDQ fit (512 px) bound them, at about 1–1.5 MB each.
Kept per worker (the `Decoder` is already per-worker state), `dct16`'s planes, `box_columns`' row and a `Resizer`'s
temp image would remove 55% of the bytes of this profile (234 MB), though only 1,525 of its blocks. The rest are outputs (the decoded image, the resized images, the
hashes) or inside crates whose API allocates.

## Every site of the scan

| # | Blocks | Bytes | Peak | Site | What, and what it would take |
|---:|---:|---:|---:|---|---|
| 1 | 612 | 9.2 KB | 6.1 KB | `scan.rs:114` `String::from_utf16_lossy(name)` | Each entry's name; 2 blocks per name (the `String` grows as it is decoded). Kept in the `DirEntry`. |
| 2 | 610 | 1.8 KB | 0 | `media.rs:55` `extension`'s `to_lowercase` | Twice per file: `kind_of` (`scan.rs:195`) and `upsert_file`'s `ext` (`catalog.rs:648`). `kind_of` can compare with `eq_ignore_ascii_case` and allocate nothing. |
| 3 | 305 | 154 KB | 0 | `catalog.rs:635` `prepare_cached("INSERT INTO files …")` | rusqlite's cache key (504 B) per file. |
| 4 | 305 | 24.1 KB | 0 | `catalog.rs:634` `[folder.key, name_key].concat()` | The order key, the stored value. |
| 5 | 305 | 12.8 KB | 0 | `browse.rs:55` `name_key`'s `Vec::with_capacity` | The name's part of it, copied into the order key and dropped; writing straight into one buffer would save the block. |
| 6 | 68 | 7.7 KB | 36 B | `cli.rs:133` `Cli::parse_from` | clap. Once. |
| 7 | 16 | 1.1 KB | 0 | `main.rs:49` `args_os().collect()` | Once. |
| 8 | 8 | 57.1 KB | 28.7 KB | `scan.rs:116` `out.push(DirEntry …)` | The folder's entries, growing by doubling. |
| 9 | 6 | 109 KB | 0 | `catalog.rs:46` `baseline()` | As in the import. |
| 10 | 6 | 364 B | 0 | `scan.rs:54` the UTF-16 path | Per open: the one folder is opened twice (`scan.rs:161` for the volume, `:175` to list it), 3 blocks each as the `Vec` grows. |
| 11 | 6 | 278 B | 0 | `scan.rs:53` `format!(r"\\?\{path}\")` | The same two opens, 3 blocks each. |
| 12 | 4 | 284 B | 0 | `catalog.rs:627` `prepare_cached("INSERT INTO folders …")` | rusqlite's cache key and first insert. |
| 13 | 4 | 84 B | 0 | `catalog.rs:312` `pragma_update` | Migration of a new catalog. Once. |
| 14 | 3 | 962 B | 0 | `catalog.rs:472` `format!` in `set_import_version` | New catalog. Once. |
| 15 | 3 | 34 B | 0 | `browse.rs:39` `to_lowercase` of each path component | Per folder. |
| 16 | 2 | 282 B | 0 | named `catalog.rs:591`, really `catalog.rs:492` | `set_borders_version`'s `format!` of the borders SQL, inlined into `Catalog::open`, so the profile names the call's line (the blocks, 94 then 188 B, are that string's, not the context message's). A new catalog sets its borders version. Once. |
| 17 | 2 | 150 B | 0 | `control.rs` `Progress::line` | Progress. |
| 18 | 2 | 123 B | 0 | `scan.rs:141` `canonicalize` | The root. Once. |
| 19 | 2 | 108 B | 72 B | `cli.rs:141` `join` | Once. |
| 20 | 2 | 84 B | 32 B | `catalog.rs:333` `Connection::open` | Once. |
| 21 | 2 | 63 B | 63 B | `scan.rs:164` `vec![root.to_string()]` | The folder stack. |
| 22 | 1 | 65.5 KB | 65.5 KB | `scan.rs:84` `vec![0u64; 8192]` | The listing buffer, per folder: the scan's peak. |
| 23 | 1 | 1.0 KB | 0 | `cli.rs:163` `println!` | std's stdout buffer, held to the end. |
| 24 | 1 | 170 B | 0 | `catalog.rs:661` `mark_missing`'s `format!` | Once per root. |
| 25 | 1 | 96 B | 0 | `cli.rs:53` clap's `roots` | Once. |
| 26 | 1 | 96 B | 96 B | `cli.rs:157` `normalized.push` | Once. |
| 27 | 1 | 88 B | 0 | `catalog.rs:654` | rusqlite's statement cache, first insert. |
| 28 | 1 | 88 B | 0 | `catalog.rs:629` | The same. |
| 29 | 1 | 74 B | 0 | `cli.rs:140` `create_dir_all` | Once. |
| 30 | 1 | 56 B | 0 | `catalog.rs:623` `prepare_cached("SELECT id FROM folders …")` | rusqlite's cache key. |
| 31 | 1 | 40 B | 0 | `browse.rs:37` `folder_key`'s `Vec` | Per folder. |
| 32 | 1 | 39 B | 0 | `scan.rs:145` `normalize_root`'s `to_string` | Once. |
| 33 | 1 | 39 B | 39 B | `scan.rs:167` `ctl.begin(…, root.to_string(), …)` | Once per root. |
| 34 | 1 | 39 B | 39 B | `scan.rs:147` `trim_end_matches(…).to_string()` | Once. |
| 35 | 3 | 4 B | 4 B | `cli.rs:108`, `:33`, `:111` | clap's default-value strings (the exit's). |
| 36 | 255 | 106 KB | 0 | `cli.rs:20`–`112` (31 sites) | clap building the `Command`, as in the import. Once. |

Per file, a scan allocates 7 blocks: the name (2), the extension twice (2), the cache key, the order key and its
name part (3). The scan is already near what it needs; rows 1, 2 and 5 are the only ones it repeats per file that
can go.

## Leads, by what they would save at scale

1. **The read pipeline's queue** (`pipeline.rs:60`): up to 52 whole files in memory, about 260–520 MB with camera
   JPEGs. Estimated, not measured: profile an import of large files to confirm.
2. **The tier-1 index's layout** (`pdq_index.rs`): 25.2 MB of empty tables at every import, and about a million
   heap blocks for a 190k-file catalog. Measure with an import into a copy of the pinterest catalog.
3. **EXIF parsed once, from our own segment walk** (`decode.rs:221`, `metadata.rs:135`): 57% of the import's
   blocks here, and two byte-by-byte reads of every file without EXIF.
4. **Per-worker scratch** for `dct16`'s planes and the `Resizer` (`pdq.rs:113`, `:117`, `resize.rs:63`): 55% of the
   bytes, few of the blocks.
5. **Small, per file:** the paths' copy (`import.rs:39`), `long_path`'s copy (`pipeline.rs:39`), the kind read as a
   `String` (`catalog.rs:714`), the hashes bound as `Vec`s (`catalog.rs:749`, `:754`), `Analysis.format` as a
   `String` (`analyze.rs:76`), the constant `format!` (`catalog.rs:766`), `extension` twice in the scan
   (`media.rs:55`), rusqlite's cache key on every `prepare_cached` (six per file in the import, one in the scan;
   inside rusqlite, and statements prepared once per batch would avoid it).
