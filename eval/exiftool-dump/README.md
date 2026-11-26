# ExifTool 13.64's tables, dumped

The first dump of ExifTool's tables and the classifier census, for the standalone Rust reimplementation of ExifTool
(`docs/plans/EXIFTOOL_RUST_REWRITE_DESIGN.md`, "ExifTool's knowledge" and its appendix). Everything here moves to the
engine's repository (`dump/`, `schema-data/`) when it is made.

The licence of the dumped data is not settled yet (the design's phase 0): `out/` is derived from ExifTool's tables,
which are under Perl's terms (GPL-1.0-or-later or Artistic-1.0).

| File         | What                                                                                                  |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `dump.pl`    | The dump: loads ExifTool with no config file, builds every table, writes `out/`                       |
| `check.py`   | The cross-check: every tag name `exiftool -list` prints must be in the dump                           |
| `census.py`  | The classifier census: which conditions and conversions match a typed shape                           |
| `census.txt` | The census report for `out/`                                                                          |
| `out/`       | The dump: JSON Lines, one file per module, plus `code.jsonl`, `lang/`, `shortcuts.jsonl`, `meta.json` |

## Running

In the container (Perl 5.40.1, the version the dump is pinned to), with the ExifTool clone at tag 13.64:

```sh
docker run --rm -v D:/GitHub/exiftool:/exiftool:ro -v D:/GitHub/photo-gallery:/work \
  -w /work/eval/exiftool-dump pg-tools sh -c '
    rm -rf out && perl dump.pl /exiftool/lib out
    perl /exiftool/exiftool -config "" -list > /tmp/list.txt && python3 -I check.py out /tmp/list.txt
    python3 -I census.py out census.txt'
```

The dump is deterministic: the same ExifTool and Perl give the same bytes, so a rerun that changes `out/` means the
input changed.
