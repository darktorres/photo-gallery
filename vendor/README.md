# Vendored crates

Crates we build from a local copy instead of crates.io. The root `Cargo.toml` routes them in with `[patch.crates-io]`
and leaves this directory out of the workspace (`exclude`), so its lints, formatting and tests are the upstream's.

## turbojpeg-sys

Copy of `turbojpeg-sys` 1.2.0 (with its `libjpeg-turbo` sources). One local change, marked in `build.rs`: libjpeg-turbo
is built with `WITH_CRT_DLL=ON`, so it uses the dynamic C runtime as the rest of the app does. Without it, its objects
link the static runtime too and the linker warns `LNK4098` (two C runtimes in one binary).

To update: copy the new version from the cargo registry over this directory, then add the `WITH_CRT_DLL` line back
to `build_vendor` in `build.rs`.
