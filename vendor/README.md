# Vendored crates

Crates we build from a local copy instead of crates.io. The root `Cargo.toml` routes them in with `[patch.crates-io]`
and leaves this directory out of the workspace (`exclude`), so its lints, formatting and tests are the upstream's.

## qt-build-utils

Copy of `qt-build-utils` 0.10.0 (cxx-qt's), so that the QML compiler (`qmlcachegen`) sees the `pg.gallery` module as
Qt's CMake build lays it out. Without that it compiles much of the QML to bytecode for the interpreter instead of C++,
and says so only with `--verbose`. The local changes are marked `photo-gallery` and are all in `register_qml_module`
(`src/lib.rs`) and `QtToolQmlTypeRegistrar::compile`:

- `qmltyperegistrar` runs before `qmlcachegen`, not after. qmlcachegen reads the module's `plugin.qmltypes`, and so
  it saw the previous build's types, or none in a clean build.
- `qmltyperegistrar` gets Qt's metatypes (`--foreign-types`, from `QT_INSTALL_ARCHDATA/metatypes`). Without them, it
  declares each Qt type that the module's types reference (`QString`, `QUrl`, `QStringList`, `QVariant`) as an opaque
  type of the module. That hides the real one: a `QString` property has no `length`, and a `QUrl` can't be assigned.
- The QML files are copied beside the module's `qmldir`, which lists them by paths relative to itself. qmlcachegen
  finds the module's own components (`FilterBar` in `Main.qml`) through it.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the three changes again, or drop the copy once upstream does
them.

## turbojpeg-sys

Copy of `turbojpeg-sys` 1.2.0 (with its `libjpeg-turbo` sources). One local change, marked in `build.rs`: libjpeg-turbo
is built with `WITH_CRT_DLL=ON`, so it uses the dynamic C runtime as the rest of the app does. Without it, its objects
link the static runtime too and the linker warns `LNK4098` (two C runtimes in one binary).

To update: copy the new version from the cargo registry over this directory, then add the `WITH_CRT_DLL` line back
to `build_vendor` in `build.rs`.
