# PNG decoder comparison

The speed and output of the PNG decoders on the library's PNGs, for [docs/PNG_DECODERS.md](../../docs/PNG_DECODERS.md).
It runs natively on Windows; nothing here needs WSL.

- `src/`: the Rust harness. Its decoders are the app's `png` fork (the pin in the root `Cargo.toml`), the same crate
  from crates.io (`png` 0.18), and zune-png 0.5.
- `pillow_bench.py`: Pillow's PNG reader (its own `zip` decoder over zlib-ng, not libpng), as a second opinion outside Rust. Run it with the venv that has Pillow and numpy
  (`D:\PhotoGallery\venv-eval`).

## To rerun

From PowerShell, in `eval\png-decoders`:

    cargo build --release
    .\target\release\png-decoders.exe compare FILE...
    .\target\release\png-decoders.exe time --secs 2 FILE...
    .\target\release\png-decoders.exe dump DIR FILE...
    D:\PhotoGallery\venv-eval\Scripts\python.exe pillow_bench.py --ref-dir DIR --secs 2 FILE...

`dump` writes the app's output for each file, and the Pillow bench checks its own output against it. The inputs are
library files, so they are read where they are and never copied into the repo.
