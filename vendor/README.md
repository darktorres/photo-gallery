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

## rusqlite

Copy of `rusqlite` 0.37.0. One local change, marked `photo-gallery` in `src/cache.rs`: `StatementCache::get` sets a
statement's cache key only when it prepares the statement. A statement taken from the cache still carries the key it
was cached under, yet `get` set a new one on every call, copying the SQL into a new `Arc<str>` for each
`prepare_cached` hit: one allocation per query, hit or miss (docs/HEAP_PROFILE.md, site 11). A test there
(`test_cache_hit_keeps_key`) checks that a hit keeps the key; it fails without the change. Upstream still has it, in
0.41.1 and on master.

To update: copy the new version from the cargo registry over this directory (except `.cargo-ok`,
`.cargo_vcs_info.json` and `Cargo.lock`), then make the change again, or drop the copy once upstream does it. Its
tests run from this directory: `cargo test --features bundled --target-dir ../../target/vendor-test`.

## dhat

dhat-rs 0.3.3 (`dhat`, MIT OR Apache-2.0), patched in phpstan-rs (`patches/dhat`, commit `13675fd3`) so that heap
profiles with whole backtraces (`trim_backtraces(None)`) take minutes rather than hours. Upstream unwinds while it
holds its spinlock, so every thread queues behind one unwind, and each new global peak sweeps every program point. The
patch takes backtraces before the lock, keeps them as return addresses resolved once at exit, records the peak lazily
and locks a `std::sync::Mutex`. Its README lists the changes; the output (`dhat-heap.json`) is upstream's.

It also walks frame pointers instead of unwind tables, but only on Linux and macOS (`build.rs`). On Windows it
unwinds with `backtrace` (`RtlVirtualUnwind`), still outside the lock. Only `gallery`'s `dhat-heap` feature uses it
(docs/PROFILING.md "Tool: dhat"). It keeps its own `Cargo.lock`, and its dependencies are pinned exactly, at the
versions the workspace's `Cargo.lock` has. Being a path dependency, it is formatted by `cargo fmt --all`: its
`rustfmt.toml` keeps it at rustfmt's defaults, as upstream's.

To update: copy phpstan-rs's `patches/dhat` over this directory (without `target/`).
