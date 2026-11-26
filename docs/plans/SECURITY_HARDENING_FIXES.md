# Fix list: hardening found by the value provenance research

> **Status (2026-11-26).** Nothing here is built. The items come from two research passes for `VALUE_PROVENANCE_IDEAS.md` (the rationale) and land before `VALUE_PROVENANCE_PLAN.md` (the type work), which does not block them. Each item is marked **read** (seen in the code, working tree of this date), **probed** (reproduced in a scratch crate outside the repo, against the repo's toolchain and SQLite build), or **to verify** (the check that settles it is named). Where `SECURITY_FLOW_REVIEW.md` already has the finding, its SEC id is given and this list adds only the fix design.
>
> Cite by symbol; line numbers drift.

## Order

High first: 1 (allocations abort the process), 2 (writes outside the chosen file), 3 (LAN reach), 4 (FFmpeg nested opens), 5 (SQLite). The rest in any order.

## 1. Allocations sized by a file's header abort the process

SEC-059 (partial), SEC-012, SEC-187. **Read.**

- `decode_jpeg` (`crates/codec/src/decode.rs`): with `MinSize::Full` the scale is 1, then `out.pixels.resize(w * h * 3)` with no reservation. A small JPEG declaring 65500×65500 asks for 12.9 GB and aborts (`Vec::resize` cannot fail softly). `find_borders` in `crates/analysis/src/analyze.rs` reaches it for every image, with no user action. The parajpeg `rgb_len()` branch, `encode.rs` `jpeg_cropped` and `webp_lossy_cropped` have the same shape.
- `crates/codec/src/animation.rs`: the GIF and APNG decoders run with `Limits::no_limits()`.
- `crates/jobs/src/pipeline.rs` `Pool::take` uses an infallible `reserve_exact(size)`.

Fix: every decoder goes through `codecs/limits.rs` `reserve` before its first allocation, as the other codecs already do, and the allocation is `try_reserve_exact`, so a refusal is a `LimitError`, not an abort. The animation decoders get the same `Limits` the stills use. `Pool::take` uses `try_reserve_exact`. This is the existing format check, applied everywhere; it adds no memory cap.

## 2. Paths built from catalog rows reach destructive writes

SEC-016 (names), SEC-015, SEC-018, SEC-072, SEC-126 (the recorded temp), SEC-083 (crop's lexical check). **Read.**

- `pg_platform::names::disk_path` joins the bytes of `files.name` and `folders.path` and checks only for NUL. `\`, `..`, `:` and drive anchors pass. Writeback rewrites the file at that path in place; `crop::save_with`'s `is_under` is lexical.
- `finish_copy` (`crates/writeback/src/lib.rs`) reads a stored temp path, copies it over the original and deletes it.
- A real scan cannot produce these names. A damaged or crafted `catalog.sqlite` can.

Fix:
- The catalog refuses, on read, a stored name holding a separator, `.` or `..`, `:`, NUL or a reserved device name (`file_path_of`). Such a row is a damaged row, reported as one.
- A recorded temp is never used as a path. It is derived again with `temp_of(original, kind)` and compared, or not stored at all.
- Writers check that the final path, resolved, is under its root (with the reparse checks of SEC-079 and SEC-128).

## 3. Yandex result URLs are fetched from any host

SEC-052, SEC-055, SEC-050/SEC-057 (`pos + 1`). **Read**; ureq APIs checked against the ureq 3.5.2 source.

`Fetcher::download` (`crates/yandex/src/fetch.rs`) GETs whatever URL the page returned, with no scheme, host or address check. Downloads start as results arrive, four at a time (`crates/app/src/yandex/images.rs`), up to 100 MB each, into the decoders. ureq's defaults: 10 redirects, and the `ALL_PROXY`, `HTTPS_PROXY` and `HTTP_PROXY` variables are read.

Fix:
- **`ResultUrl`** in pg-yandex, private field, built by the parser: http or https, a host, no userinfo. `Fetcher::download` takes `&ResultUrl`.
- **A resolver that admits public addresses only.** Implement `ureq::unversioned::resolver::Resolver` around `DefaultResolver`; refuse the host when any resolved address is not public, with `ureq::Error::Other`. Install with `Agent::with_parts(config, DefaultConnector::default(), PublicOnly)`. Every redirect hop resolves through it (`run.rs` `connect`), and the TCP connector dials exactly the checked addresses, so DNS rebinding cannot get past it. IP literals and odd forms (a decimal integer host) are covered, since the check is on the resolved result.
- **`.proxy(None)`.** With a proxy set, ureq does not resolve the target, so the policy would silently stop. A proxy, if ever wanted, is an explicit preference.
- **`.max_redirects(5)`** on both agents; keep `max_redirects_will_error(true)`.
- **`.https_only(true)`** on the search `Client` only. Many result images are http-only, and https does nothing against a hostile host.
- **No host allowlist**: results are on third-party hosts.
- **The address table**, written out (`IpAddr::is_global` is unstable), with a test for each row. IPv4: 0/8, 10/8, 100.64/10, 127/8, 169.254/16, 172.16/12, 192.0.0/24, 192.0.2/24, 192.88.99/24, 192.168/16, 198.18/15, 198.51.100/24, 203.0.113/24, 224/4, 240/4. IPv6: `::/128`, `::1`, `::/96`, `::ffff:0:0/96` (check the embedded v4 with `to_ipv4_mapped`), `64:ff9b::/96` and `64:ff9b:1::/48` (embedded v4), `100::/64`, `2001::/32` (refuse), `2001:db8::/32`, `2002::/16` (embedded v4), `fc00::/7`, `fe80::/10`, `fec0::/10`, `ff00::/8`.
- The `unversioned` module is outside ureq's semver promise; re-check it on each ureq bump.
- `s.pos + 1` in `crates/app/src/yandex.rs` becomes `checked_add` (the parser accepts `u32::MAX`).

## 4. FFmpeg can open nested inputs

**Read**; checked against FFmpeg n9.0.2 and qtmultimedia's 6.10 and dev branches.

Release builds link a static FFmpeg built with `--disable-network --enable-protocol=file` and five demuxers (`tools/build_static_ffmpeg.py`). A dev build against a full FFmpeg (`FFMPEG_DIR`) is not limited: HLS, concat and `image2` with `split_planes` open further inputs through `AVFormatContext::io_open`. `protocol_whitelist=file` is not the fix: `file` is exactly the nested local read to stop.

pg-video, in `Input::open` (`crates/video/src/ffi.rs`), before `avformat_open_input`:
1. `(*ctx).io_open = Some(refuse)`, where `refuse` returns `AVERROR(EPERM)`; leave `io_close2` at its default. The main input never calls `io_open`, since `s->pb` is set, and nested contexts inherit the callback. This holds whichever FFmpeg is linked.
2. `format_whitelist=mov,matroska,avi,mpegts,image2` and `codec_whitelist=h264,hevc,vp9,mpeg4,mjpeg` in the options dictionary. In `Decoder::open`, check the codec id against a Rust enum before `avcodec_find_decoder`.
3. After the open, a non-empty options dictionary (`av_dict_count`) is an error, so a misspelled option cannot silently do nothing. Add `av_dict_set`, `av_dict_count` and `av_dict_free` to `Api` in `load.rs` and to `linked()`.
4. An `interrupt_callback`, so a job's stop interrupts a slow probe.

Qt Multimedia's own context (`crates/desktop/cpp/video_source.cpp`, `QMediaPlayer::setSourceDevice`) has no `io_open` hook. Set `QT_FFMPEG_PROTOCOL_WHITELIST` to a value naming no protocol (device playback needs none) with `qputenv` in C++ before `QGuiApplication`. **To verify:** that Qt 6.12.0's `qffmpegmediadataholder.cpp` still reads this variable.

## 5. SQLite opens crafted files with its defaults

**Probed** (`scratchpad\sqlprobe`, SQLite 3.54.1 from the repo's rusqlite fork).

Today: `trusted_schema` on, DEFENSIVE off, double-quoted strings on, URIs on (`SQLITE_USE_URI`, and `OpenFlags::default()` includes `SQLITE_OPEN_URI`); FTS3, FTS5, RTREE, DBSTAT and extension loading compiled in and unused. Measured:

- A crafted view `files` in an old catalog runs `pg_contains` during the rebuild's carry. With `trusted_schema` off it is refused.
- A generated column that calls an app function makes the `ATTACH` itself fail once `trusted_schema` is off (DatabaseCorrupt). The rebuild must report this as `Carried::Nothing`, and `Catalog::prepare`'s open path must be retested.
- **An FTS5 virtual table named `analysis` in the old file is read even with `trusted_schema` off**, so a crafted file reaches the FTS5 and RTREE code.
- A carried `sqlite_sequence` value of `i64::MAX` makes every later insert fail with "database or disk is full".
- The real `baseline.sql` creates and runs cleanly under DEFENSIVE, `trusted_schema` off, DQS off and both `ATTACH_*` settings off (triggers, `thumb_*` views, `pg_contains`/`pg_ext`, `quick_check`, `integrity_check`). DQS off rejects `key = "seq"`; fix any such literal.
- `ENABLE_VIEW` off blocks the app's own views, so it is not usable.

Fix:
- **One opener**, `pg_dbfile::open(path, mode)`, for every connection. It omits `SQLITE_OPEN_URI` and sets `SQLITE_DBCONFIG_DEFENSIVE`, `TRUSTED_SCHEMA` off, `DQS_DDL` and `DQS_DML` off, `SQLITE_LIMIT_ATTACH` 1, and on connections that attach a file, `ATTACH_WRITE` and `ATTACH_CREATE` off. `clippy.toml` bans `rusqlite::Connection::open*` so tests run hardened too.
- **The fork's build** (`photo-gallery-defaults`): drop `-DSQLITE_USE_URI`; add `-DSQLITE_TRUSTED_SCHEMA=0`, `-DSQLITE_DQS=0`, `-DSQLITE_OMIT_LOAD_EXTENSION`; drop FTS3, FTS5, RTREE, DBSTAT and SOUNDEX by editing the `ENABLE_*` list in `build.rs`, not by `-U` order.
- **Function flags**: `pg_ext` and `pg_contains` are `SQLITE_DIRECTONLY` (no schema uses them); `path_name_key` stays `SQLITE_INNOCUOUS` (the `files_mn` index needs it).
- **The rebuild**: carry only tables with `type = 'table' AND rootpage > 0` in `carried.sqlite_schema`; run `PRAGMA carried.quick_check` with the app's functions registered first; `cell_size_check` on the rebuild connection; clamp the carried `sqlite_sequence` to the largest carried rowid.
- `Layout` paths are made absolute (a relative `PG_CATALOG_DIR` is SEC-028).

## 6. Directory listing walks a buffer without bounds checks

**Read.** `Lister::each` (`crates/platform/src/dir.rs`) follows `NextEntryOffset` and `FileNameLength` of each `FILE_ID_BOTH_DIR_INFO` without checking them against the buffer length. On a network drive the buffer holds the server's answer.

Fix: before reading a record, check offset + header + `FileNameLength` ≤ `len` and that `NextEntryOffset` advances; a bad chain ends the listing with an error.

## 7. Local time for past dates is off by an hour

**Probed** (`scratchpad\tzprobe`). SQLite's `'localtime'` goes through the C runtime, which applies today's zone rules to every date. In `E. South America Standard Time`, 2015-01-15 12:00 UTC gets −3 h; Windows' `TimeZoneInfo` gives −2 h (DST until 2019). Wrong: every UTC-sourced `taken` from a pre-2019 summer, the mtime fallback in `key_sql` (Taken), the day bounds in `local_start_sql`, and the `TAKEN_ZONE_QUERY` fingerprint, which samples with the same function.

Fix: one registered function, `pg_localtime(instant)`, from `pg_platform`, using `GetDynamicTimeZoneInformation` and `SystemTimeToTzSpecificLocalTimeEx` (historical rules), used by `store_analysis`, `set_taken_zone`, `local_start_sql` and `key_sql`. The fingerprint becomes the zone key name plus a hash of `pg_localtime` offsets sampled across years. Changing it re-derives `taken` for UTC-sourced rows (`set_taken_zone`).

## 8. An unknown video length is stored as 0

**Read.** `probe` (`crates/video/src/lib.rs`) does `input.duration().max(0) / 1000`, so FFmpeg's unknown (`AV_NOPTS_VALUE`, `i64::MIN`) becomes 0 ms. `NO_DURATION` is reached only for a negative length or one over 49.7 days, and the comments in `analyze.rs` and `writers.rs` say the opposite. Fix: unknown is `None` from `probe` and NULL in the catalog; correct the comments. The type is `Option<DurationMs>` in plan phase 1; the behavior fix does not wait for it.

## 9. QML entries that do not check their arguments

**Read.**

- `Viewport.fitSource(key, w, h)` (`crates/desktop/src/viewport.rs`): `view_of` clamps regions to 8192 but not `View::Fit` (`crates/client/src/image.rs`), and QML can write any `image://full/...` id. An allocation sized by a QML number. Clamp `Fit` as regions are.
- `GalleryModel.relayout` and the Viewport inputs (`setMedia`, `setWindow`, `zoomAt`, `pan`): no finiteness check. Refuse NaN and infinity at entry.
- `step_preview(by)` (`crates/desktop/src/yandex.rs`): `preview_item + by` is unchecked `i32` math, a panic across FFI in dev builds. Use checked math.
- `file_id_of` (`crates/desktop/src/rows.rs`) accepts 0; it should accept `1..=2^53`. `set_file_id_and_play` (`animation.rs`) stores the raw `i64` and checks `<= 0` only later, in `restart`; convert at entry.

## 10. Untrusted text shown or logged

**Read.**

- The Yandex status `TextEdit` (`YandexPanel.qml`) shows URL text from error wording and sets no `textFormat`. Qt 6's default is plain text; set `TextEdit.PlainText` explicitly. Extend the `crates/desktop/src/qml_text.rs` test to `TextEdit` and to Controls' `text:` (`ItemDelegate`, `MenuItem`, `Chip`, fed from folder and view names). **To verify:** whether Controls' `text:` can render rich text.
- `notice::tidy` removes control characters but not bidi overrides (U+202E), and notice path fields skip it; the stderr fallback `Record::plain` is raw. One `display_text()` for every untrusted string, removing control and bidi-format characters, used by notices and the fallback.

## 11. ExifTool output

SEC-082. **Read.** The output is matched with `contains` (`crates/exiftool/src/lib.rs`) and read with no cap per line. Fix: run with `-j` and deserialize into a struct; cap a line at the longest answer the protocol can give.

## 12. Small arithmetic

**Read.**

- `Detection::cut` (`crates/model/src/detection.rs`) computes `self.width - r` unchecked; nothing enforces that `content` lies inside the image. Checked math now; `ContentBox` in plan phase 4.
- `png_grid_search` (`crates/container/src/lib.rs`): `step = len.saturating_add(12)` can saturate to `usize::MAX`, then `run_at + (lo + 1) * step` overflows. 32-bit targets only. Use checked math.
- GIF frame counting (`images += 1`, `crates/metadata/src/container.rs`) is unchecked, unlike WebP's `saturating_add`. Use `saturating_add`.

## 13. Comments that say the wrong thing

**Read.** The `Analysis.orientation` doc says "EXIF orientation the file asks for"; the value is the file's orientation with the catalog turns composed in (`Decoder::dimensions`, `analyze_video`). The `NO_DURATION` comments (item 8). `baseline.sql`'s `pdq_pairs` comment says "dihedral variant"; the writer stores a Hamming distance.
