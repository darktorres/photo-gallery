//! On Linux, the static libraries' symbols stay out of the binaries' dynamic symbol table (`--exclude-libs,ALL`).
//! turbojpeg-sys links libjpeg-turbo statically, built with the libjpeg 6.2 API, and libvips loads the system's
//! libjpeg.so.8. A symbol a shared library uses is exported from the executable when the executable defines it, so
//! without this libvips's `jpeg_*` calls bind to the static copy and fail ("Wrong JPEG library version: library is 62,
//! caller expects 80"). A link argument here, not a rustflag in .cargo, since an environment RUSTFLAGS (the container
//! sets one) replaces those.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
    }
}
