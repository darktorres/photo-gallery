# Photo Gallery

![The app icon: mascot 03, a crab in a red Rust bucket](crates/desktop/resources/app-icon.png)

A desktop photo gallery for very large libraries, built around near-duplicate detection: many crops and edits of the
same picture, and the same pin saved at different sizes. Non-destructive by design: nothing is deleted automatically.

- **Rust** for the core: scanning, the SQLite catalog, decoding and thumbnails, duplicate detection.
- **Qt 6.12 and QML** for the window, through cxx-qt.
- **Windows only** for now.

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

- `docs/PLAN.md`: goals, constraints and milestones.
- `docs/REFACTOR_PLAN.md` and `docs/REFACTOR_DESIGN.md`: how the code was restructured, and why.
- `crates/README.md`: the crates, their layers, and the build in detail.
- `docs/FINDINGS.md`: measurements and what they showed.
- `docs/PROFILING.md`: the callgrind benches and the instruction-count gate.
