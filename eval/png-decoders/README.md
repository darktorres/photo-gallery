# PNG decoder comparison

The speed, heap traffic and output of the PNG decoders on the library's PNGs, on generated and real 16-bit inputs,
for [docs/temp/PNG_DECODERS.md](../../docs/temp/PNG_DECODERS.md). It runs natively on Windows; nothing here needs WSL.

- `src/`: the Rust harness. Its decoders are the app's `png` fork (the pin in the root `Cargo.toml`), the same crate
  from crates.io (`png` 0.18), and zune-png 0.5. It has a counting global allocator for the `allocs` command, and
  `inflate` and `inflate-stream`, which time fdeflate (the fork's inflater) on a file's IDAT stream alone: `inflate` in
  one call, `inflate-stream` in 16 KiB input pieces into a 96 KiB window that is rolled back to its last 32 KiB.
  `PNG_INFLATE_NO_CHECKSUM=1` times the inflate without the zlib checksum, as the app's fork runs it (the C harness's
  zlib then reads raw deflate). Set it for all three inflate commands, or the figures are not comparable with the app.
- `harness/`: the C++ harness (MSVC, CMake + Ninja): libpng 1.6.59, libspng 0.7.4, Wuffs d28ecd4, all with zlib 1.4
  (static). `win_build.py` clones and builds them at their pinned commits under `$PG_ROOT\eval\png-decoders`. Its
  commands are `time`, `compare`, `allocs`, `inflate` (zlib, one call), `inflate-stream` (zlib, streamed as the Rust
  one is), `inflate-wuffs` (Wuffs' zlib decoder, streamed and in one call) and `dump` (each decoder's raw output, for
  finding where it differs).
- `pillow_bench.py`: Pillow's PNG reader (its own `zip` decoder over zlib-ng, not libpng), as a second opinion. It
  skips 16-bit files, which Pillow reads at 8 bits. Run it with the Python that has Pillow and numpy
  (`$PG_ROOT\venv-eval`, see [eval/README.md](../README.md)).
- `make_inputs.py`: writes the seven generated inputs (16-bit, Adam7, grayscale, APNG) under
  `$PG_ROOT\eval\png-decoders\inputs`, and with `--verify-dump DIR` checks the app's 16-bit output against its source.
- `make_variants.py`: the tRNS variants (including 16-bit grey and the real photograph's key), the filter-0 variants and
  the text-free copies of the library files, under `inputs-variants` and `inputs-notext`. The 16-bit photograph's colour-key variant takes its key from the app's dump
  of that photo, so run `dump` on `inputs-real16` (into `ref`) first.
- `make_real16.py`: four real 16-bit photographs from the imagecompression.info 16-bit RGB set, read from its zip
  (`--zip PATH`, the archive downloaded from https://imagecompression.info/test_images/rgb16bit.zip), under
  `inputs-real16`. Their terms are in the archive's readme.txt; they allow redistribution with attribution.

## To rerun

From PowerShell, with `$env:PG_ROOT` set (PowerShell doesn't read `.env`):

    # the Rust harness, in eval\png-decoders
    cargo build --release
    .\target\release\png-decoders.exe compare FILE...
    .\target\release\png-decoders.exe time --secs 2 FILE...
    .\target\release\png-decoders.exe allocs FILE...
    .\target\release\png-decoders.exe inflate --secs 2 FILE...
    .\target\release\png-decoders.exe inflate-stream --secs 5 FILE...
    .\target\release\png-decoders.exe dump DIR FILE...

    # the C harness
    python eval\png-decoders\win_build.py
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe compare DIR FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe time --secs 2 FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe allocs FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe inflate --secs 2 FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe inflate-stream --secs 5 FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe dump DIR FILE...

    # Pillow
    & "$env:PG_ROOT\venv-eval\Scripts\python.exe" pillow_bench.py --ref-dir DIR --secs 2 FILE...

    # inputs
    python eval\png-decoders\make_inputs.py
    python eval\png-decoders\make_inputs.py --verify-dump DIR
    python eval\png-decoders\make_variants.py
    python eval\png-decoders\make_real16.py --zip PATH\rgb16bit.zip

`dump` writes the app's output for each file (DIR/NAME.raw, and DIR/NAME.txt with "width height channels
bytes-per-sample"). The C harness, the Pillow bench and `compare` check their output against that DIR; the reference
is made from the same file stems, so the text-free library copies use the same reference as the originals. The inputs
are library files and generated files, so they are read where they are and never copied into the repo.

Timings should be taken on a quiet machine. The harnesses run the decoders back to back on each file, so a file's
ratios hold up under load better than its absolute times do. Absolute times still moved 20–40% between runs in the
study, so compare ratios within one run, and use a budget of at least 5 s for the inflate-stream numbers (a 2 s run
gave the real photographs' fdeflate times 21–33% high).
