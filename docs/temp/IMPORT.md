# Import

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [TEST_SET.md](TEST_SET.md); the old titles are listed in [FINDINGS.md](FINDINGS.md).

## M1 import (Rust, `crates/`, 2026-10-30)

Measured on `dups2` (5,937 files, copy on D:), with GPU benchmarks running alongside, so
the speeds are lower bounds. The full 190k-image run was stopped before it finished.

- **Speed:** 341 files/s on 12 workers (14 ms per image on one thread: decode 8.7, PDQ 2.3, Lanczos to
  512 px 1.7, rest < 0.2). The pipeline is CPU-bound, as M0 predicted.
- **PDQ matches M0.** The Rust port (dihedral variants derived from the DCT, as Meta's hasher does) is
  bit-exact with the `pdqhash` crate. Against `eval/pdq.py`'s hashes: 74% identical, 98% within 4 bits, max
  14 (the DCT-scaled decode differs slightly from PIL's full decode). Labeled-pair recall at d ≤ 0 / 8 / 16 /
  22 / 31: **30.8 / 66.0 / 75.4 / 80.4 / 83.2%** (M0: 32.6 / 65.7 / 74.8 / 80.1 / 82.8%). No cross-group pair
  at d ≤ 31.
- **Tiers 0a/0b:** 0 byte-identical groups and 234 pixel-identical groups (468 files), as expected for
  `.mozjpeg.jpg` re-encodes (see "Test set").
- **Resumability:** an import killed after 1,500 files and rerun ends with the same 2,784 pairs as an
  uninterrupted run. Renamed and moved files keep their analysis (file ID identity); a deleted file is marked
  missing; a touched file is re-analyzed alone.
- **Thumbnail task:** 415 files/s at normal priority. Windows' background mode (`PROCESS_MODE_BACKGROUND_BEGIN`)
  dropped that to 64 files/s, because it also throttles I/O on an idle disk, so the task uses idle *CPU*
  priority instead (330 files/s with other load on the machine).
