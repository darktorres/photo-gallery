# PNG decoder comparison

The speed, heap traffic and output of the PNG decoders on the library's PNGs and on generated inputs, for
[docs/temp/PNG_DECODERS.md](../../docs/temp/PNG_DECODERS.md). It runs natively on Windows; nothing here needs WSL.

- `src/`: the Rust harness. Its decoders are the app's `png` fork (the pin in the root `Cargo.toml`), the same crate
  from crates.io (`png` 0.18), and zune-png 0.5. It has a counting global allocator for the `allocs` command.
- `harness/`: the C++ harness (MSVC, CMake + Ninja): libpng 1.6.59, libspng 0.7.4, Wuffs d28ecd4, all with zlib 1.4
  (static). `win_build.py` clones and builds them at their pinned commits under `$PG_ROOT\eval\png-decoders`.
- `pillow_bench.py`: Pillow's PNG reader (its own `zip` decoder over zlib-ng, not libpng), as a second opinion. It
  skips 16-bit files, which Pillow reads at 8 bits. Run it with the Python that has Pillow and numpy
  (`$PG_ROOT\venv-eval`, see [eval/README.md](../README.md)).
- `make_inputs.py`: writes the seven generated inputs (16-bit, Adam7, grayscale, APNG) under
  `$PG_ROOT\eval\png-decoders\inputs`, and with `--verify-dump DIR` checks the app's 16-bit output against its source.

## To rerun

From PowerShell, with `$env:PG_ROOT` set (PowerShell doesn't read `.env`):

    # the Rust harness, in eval\png-decoders
    cargo build --release
    .\target\release\png-decoders.exe compare FILE...
    .\target\release\png-decoders.exe time --secs 2 FILE...
    .\target\release\png-decoders.exe allocs FILE...
    .\target\release\png-decoders.exe dump DIR FILE...

    # the C harness
    python eval\png-decoders\win_build.py
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe compare DIR FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe time --secs 2 FILE...
    $env:PG_ROOT\eval\png-decoders\build\harness\png-c-harness.exe allocs FILE...

    # Pillow
    & "$env:PG_ROOT\venv-eval\Scripts\python.exe" pillow_bench.py --ref-dir DIR --secs 2 FILE...

    # inputs
    python eval\png-decoders\make_inputs.py
    python eval\png-decoders\make_inputs.py --verify-dump DIR

`dump` writes the app's output for each file (DIR/NAME.raw, and DIR/NAME.txt with "width height channels
bytes-per-sample"). The C harness and the Pillow bench check their own output against that DIR. The inputs are library
files and generated files, so they are read where they are and never copied into the repo.
