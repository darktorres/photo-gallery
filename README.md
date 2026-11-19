# Photo Gallery

<!-- markdownlint-disable-next-line MD033 -->
<p align="center"><img src="crates/desktop/resources/app-icon.png" alt="The app icon: mascot 03, a crab in a red Rust bucket" width="160"></p>

A desktop photo gallery for very large libraries, built around near-duplicate detection: many crops and edits of the
same picture, and the same pin saved at different sizes. Non-destructive by design: nothing is deleted automatically.

- **Rust** for the core: scanning, the SQLite catalog, decoding and thumbnails, duplicate detection.
- **Qt and QML** for the window, through cxx-qt (the Qt version is the Justfile's `qt_version`).
- **Windows only** for now.

The project page: <https://darktorres.github.io/photo-gallery/> (its source is `site/`).

## Building

The compiler is pinned in `rust-toolchain.toml`, so `rustup` installs the right one on first use. The Qt kit, the FFmpeg
build and ExifTool are described in `crates/README.md` ("Build"). This machine's paths go in `.env` (copy `.env.example`):
`just doctor` says what is missing, and `just setup` fetches the pinned FFmpeg. The commands are in the `Justfile`, which
needs [just](https://just.systems):

```text
just            # lists the commands
just lint       # formatting, clippy, dependency checks, Python and Markdown, QML
just test       # the Rust tests
just qmltest    # the QML tests
just deploy     # the release build, with Qt's DLLs and QML modules next to gallery.exe
```

## Documentation

- `docs/plans/PLAN.md`: goals, constraints and milestones.
- `docs/reference/REFACTOR_DESIGN.md`: how the code was restructured, and why. Open refactor work is in `docs/plans/REFACTOR_LEDGER.md`.
- `crates/README.md`: the crates, their layers, and the build in detail.
- `docs/eval/`: the model and verifier measurements: `MODELS.md` (models and accelerators), `VERIFIER.md` (PDQ, the verifier and grouping), and `TEST_SET.md` (hardware, test set and labels).
- `docs/reference/`: facts that stay true, e.g. `IO.md` (HDD read order, buffers, long paths), `START_PROFILE.md` (where the start goes, thread by thread) and `THUMBNAILS.md` (thumbnail resize, encode and priority measurements) and `JPEG_DECODERS.md` (which JPEG decoder the app uses, and why) and `REVERSE_SEARCH.md` (Yandex reverse image search: what it returns and how the port reads it).
- `docs/webclient/`: the web client's design, tech decisions and sync notes (`WEB_CLIENT*.md`, `INCREMENTAL_SYNC.md`), and `scan-optimizations-validation.md`, the scan measurements.
- `docs/plans/PROFILING_PLAN.md`: what the profiling still has to measure, and the leads not acted on.
- `docs/plans/OPEN_ITEMS.md`: the open items from the measurements, each checked against the code.
