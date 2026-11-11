# Test set and environment

Hardware, the test set and its labels, and the eval tooling. Measurements elsewhere use the quick-test subset (`eval/make_subset.py`: all labeled files plus 3,000 random others, ~9.3k images) unless they say otherwise. Raw outputs live in `D:\PhotoGallery\eval_smoke` (subset) and `D:\PhotoGallery\eval` (full set, not run yet). Browse them with `eval/viewer.py`.

## Environment

| Item | Value |
|---|---|
| CPU / RAM | i7-8700 (6C/12T, AVX2), 64 GB |
| GPU | RTX 2060, 6 GB, sm_75 |
| Library | E: 22 TB Exos HDD (21.4 TB used); C: has ~60 GB free after the 2026-10-28 cleanup (keep everything off it) |
| Fast storage | D: 2 TB NVMe |
| Eval Python | `D:\PhotoGallery\venv-eval` (torch 2.15 + CUDA 13.4 since 2026-10-30, NVIDIA driver 621.01); global Python's torch is CPU-only |
| Port 8765 | Used by a local test server outside the repo; the viewer takes the next free port (8766) |

## Test set (2026-10-28)

a copy of the Pinterest folder, copied to `D:\PhotoGallery\datasets\pinterest`.

- 189,528 files, 30 GB, average 170 KB (p50 126 KB, p99 548 KB).
- 98.9% images (187,614 `.jpg`), plus ~1.7k videos (excluded for now).
- **98.4% of files are `.mozjpeg.jpg` re-encodes**, so byte-identical matching finds few
  duplicates. The decoded-pixel hash and the perceptual tiers matter more.

### Labels

- `dups2/<NNN>/`: matched duplicate groups. After fixes: **678 groups, 1,363 files, 702 pairs**.
- `dups2/*` (loose): **4,574 files**, each with a duplicate among the other loose files, unsorted.
- `similiar_but_not_dups/`: **370 files** that other tools wrongly flagged; ~68k non-duplicate pairs.

**Label fixes (2026-10-28, applied on both D: and E:, verified identical):** 7 folders each
held two unrelated duplicate pairs. The second pair was moved to a new folder:
075→672, 076→673, 127→674, 174→675, 175→676, 266→677, 281→678.

**2026-10-29:** 090 → 679 (two pairs of different frames of one shoot; E: done, D: after the k-NN
run; `files.txt` paths patched in place so embeddings stay row-aligned). Now 679 groups and
698 labeled pairs.

**Still open:** 9 single-file folders (230, 348, 416, 417, 423, 530, 538, 540, 628). Their
partner is not among the loose `dups2` files (best match ≤ 0.40), so it may be in `Photos/`
or deleted. Finding it needs a full-set run.


## Eval viewer: port collision (bug)

- **Viewer port collision:** Python's `HTTPServer` sets `SO_REUSEADDR`, which on Windows lets
  it bind a port another server already uses and lose requests silently. The viewer now
  binds with `SO_EXCLUSIVEADDRUSE` and falls back to the next free port.
