# Heap profile: scan, import and the passes (dhat, 2026-11-01 and 2026-11-02)

Every Rust heap allocation of `gallery scan`, `gallery import` and the borders and thumbnail passes, measured with dhat
before the fixes and after three rounds of them. The method is in [PROFILING.md](PROFILING.md) ("Tool: dhat heap
profiles"). Qt's, FFmpeg's and libjpeg-turbo's allocations are not in these numbers.

**The profiles are no longer on disk** (`D:\PhotoGallery\scratch_dhat`), so the totals below cannot be re-derived; they
are the record. The per-site tables that went with them were removed on 2026-11-13: their file and line references had
moved, most of the fixes they proposed are in the code, and the full text is in git history (`ce1622a`). What is still
open is in [OPEN_ITEMS.md](../plans/OPEN_ITEMS.md).

## Workload and runs

- **Scan and import:** 305 JPEGs of 5.3 MB in all, none with EXIF or XMP, into a catalog with no earlier analyses.
  `gallery scan` into a new catalog, then `gallery import --workers 1` (one worker gives the same counts on every run)
  and the default 12 workers for the peak.
- **Borders and thumbnails:** `gallery borders --readers 1 --workers 1` and `gallery thumbs --height 256 --readers 1
  --workers 1 --foreground` on an 898-file corpus (474 MB: camera JPEGs of 6000 × 4000, PNG, WebP, GIF, TIFF, videos,
  and one 142 MB PNG). The corpus is not kept with the repo.

The samples do not contain EXIF-bearing files at scale, large files in an import, or a catalog with existing hashes.
The costs those would add are estimates from the code, not measurements.

## Totals

| Run                              | Before (2026-11-01)    | After the first fixes | After the second | After the third |
|----------------------------------|------------------------|-----------------------|------------------|-----------------|
| Scan, 305 files: allocated       | 544 KB in 2,472 blocks | 198 KB in 836         | 201 KB in 535    | 201 KB in 543   |
| Scan: peak                       | 101 KB in 318          | 101 KB in 321         | 101 KB in 321    | 101 KB in 321   |
| Import, 1 worker: allocated      | 425.6 MB in 167,437    | 12.8 MB in 1,718      | 8.4 MB in 1,275  | 5.8 MB in 440   |
| Import, 1 worker: peak           | 27.6 MB in 1,041       | 7.2 MB in 924         | 7.2 MB in 662    | 3.6 MB in 128   |
| Import, 12 workers: allocated    | 425.6 MB in 167,454    | 53.2 MB in 2,400      | 46.4 MB in 1,500 | 43.2 MB in 841  |
| Import, 12 workers: peak         | 33.2 MB in 1,380       | 32.1 MB in 995        | 31.0 MB in 728   | 28.5 MB in 359  |
| Borders, 898 files: allocated    | 5,946 MB in 559,711    | 1,972 MB in 26,861    | 830 MB in 2,637  | 658 MB in 513   |
| Borders: peak                    | 845.6 MB in 1,807      | 500.4 MB in 942       | 496.3 MB in 89   | 496.5 MB in 52  |
| Thumbnails, 898 files: allocated | 5,413 MB in 479,146    | 1,998 MB in 24,032    | 879 MB in 3,443  | 698 MB in 1,165 |
| Thumbnails: peak                 | 846.7 MB in 1,932      | 502.2 MB in 1,046     | 502.5 MB in 160  | 505.6 MB in 280 |

At exit, every run holds 1–2 KB in 4–9 blocks: thread state and statics, by design.

The per-file figures for the import: 1.40 MB in 549 blocks before the fixes, 27 KB in 4.2 after the second round, and
19 KB in 1.4 blocks after the third. The one-worker import's peak after the third round is the workspace's buffers
(PDQ planes, decode, ThumbHash planes, resizer) and the read buffers.

The passes' peak is one file: the 142 MB PNG, read whole (141.9 MB) and decoded to 348.4 MB (1216 × 95,513 × 3).
The read buffer is freed after use.

## What the rounds changed

**First round (2026-11-02).** Per-worker workspaces for the PDQ planes, the decode, the resizer and the ThumbHash planes,
kept across files. Metadata walks without per-walk allocations. Our own EXIF reader, so the file's EXIF is no longer read
through kamadak-exif twice. A read pool bounded in bytes. A flat tier-1 index: the 25.2 MB of empty tables at every
import is gone. Statements prepared once per writer. The borders' `gray3`, `DetailSums`, `SolidLines` and the thumbnail pass's per-call coefficients are gone.

**Second round.** The read pool replaces a too-small idle buffer. Work lists keep their paths in one arena. Scan names
are decoded at their exact size. The ThumbHash planes are sized once. Placeholder hashes are read in place. The folder
tree's buffers are reserved. Metadata reads use kept buffers. The non-JPEG decoders are driven by our own front end
instead of the `image` crate's per-file decoder and output buffer. `import --again` was added to the command set.

**Third round.** The read pool is bounded by files in flight as well as by bytes, and its buffers are reused and grown in
place. The raw channel is bounded. The readers open files without std's UTF-16 copy of the path. The ThumbHash is inline.
The thumbnail pass recycles its JPEG buffers per batch. Border sides are held inline. `median_f64` runs without a sort.
gif no longer collects its XMP unless asked. BigTIFF is recognized, the one behaviour change in these rounds. The
command line is read once, and every connection closes before exit.

Open items from these profiles, and the items they did not cover, are in [OPEN_ITEMS.md](../plans/OPEN_ITEMS.md).
