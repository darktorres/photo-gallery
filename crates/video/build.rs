//! FFmpeg's C types, constants and version numbers for pg-video, generated from the headers in `$FFMPEG_DIR\include`
//! (set in `.cargo/config.toml`), so no struct layout is written by hand. pg-video doesn't link FFmpeg: it loads the
//! DLLs at run time (src/ffi.rs), and the functions it calls are looked up by name there.
//!
//! On Windows the FFmpeg DLLs are also copied beside the app's binary: Qt Multimedia loads the same DLLs to play
//! videos (same major version), so gallery.exe needs them beside it (crates/README.md "Build"). The test executables
//! don't: pg-video finds them by their full path.

#![expect(clippy::panic, reason = "a build script fails by panicking, and cargo prints the message")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What Qt Multimedia loads; avdevice and avfilter are used by neither.
const DLLS: &[&str] = &["avcodec", "avformat", "avutil", "swresample", "swscale"];

/// The C types pg-video uses. Enums come out as constants, so their values are the C ones.
const TYPES: &[&str] = &[
    "AVCodec",
    "AVCodecContext",
    "AVCodecParameters",
    "AVDictionary",
    "AVDictionaryEntry",
    "AVFormatContext",
    "AVFrame",
    "AVInputFormat",
    "AVPacket",
    "AVPacketSideData",
    "AVRational",
    "AVStream",
    "SwsContext",
];

/// The constants pg-video uses: no-timestamp, the log level, the enum values, and each library's major version.
const VARS: &[&str] = &[
    "AV_NOPTS_VALUE",
    "AV_LOG_FATAL",
    "AV_PKT_DATA_DISPLAYMATRIX",
    "AVMEDIA_TYPE_VIDEO",
    "AV_PIX_FMT_RGB24",
    "AVCOL_SPC_.*",
    "AVCOL_RANGE_.*",
    "LIB(AVCODEC|AVFORMAT|AVUTIL|SWRESAMPLE|SWSCALE)_VERSION_MAJOR",
];

/// A failure is an error (cargo prints it and the build stops) or a panic with its message.
fn main() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let dir = std::env::var_os("FFMPEG_DIR").unwrap_or_else(|| {
        panic!("FFMPEG_DIR is not set: the FFmpeg headers and DLLs are found through it (.cargo/config.toml sets it)")
    });
    let dir = Path::new(&dir);
    let include = dir.join("include");
    if !include.join("libavcodec").join("avcodec.h").is_file() {
        panic!(
            "no FFmpeg headers in {}: run `just setup` to fetch the pinned build, or set FFMPEG_DIR in .env to an \
             FFmpeg 9.0 shared build",
            dir.display()
        );
    }
    println!("cargo:rerun-if-changed={}", include.display());
    generate_bindings(&include)?;

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let bin = dir.join("bin");
        // Compiled in, so the loader can find the DLLs when no copy is beside the executable.
        println!("cargo:rustc-env=PG_FFMPEG_BIN={}", bin.display());
        println!("cargo:rerun-if-changed={}", bin.display());
        copy_dlls_beside_app(&bin)?;
    }
    Ok(())
}

/// `OUT_DIR/ffmpeg.rs`: the types and constants from the headers.
fn generate_bindings(include: &Path) -> Result<(), String> {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set: cargo sets it for build scripts")?);
    let headers = [
        "libavcodec/avcodec.h",
        "libavformat/avformat.h",
        "libavutil/dict.h",
        "libavutil/error.h",
        "libavutil/frame.h",
        "libavutil/log.h",
        "libswscale/swscale.h",
    ];
    // No `Default` impls (their bodies are `unsafe { zeroed() }`, which the workspace's undocumented-unsafe lint flags
    // in generated code), no layout tests (bindgen writes `#[allow]` on them, which `allow_attributes` rejects), and no
    // doc comments (the headers' examples would become doctests that don't compile).
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("-I{}", include.display()))
        .derive_default(false)
        .layout_tests(false)
        .generate_comments(false);
    for header in headers {
        builder = builder.header(include.join(header).to_string_lossy());
    }
    for ty in TYPES {
        builder = builder.allowlist_type(ty);
    }
    for var in VARS {
        builder = builder.allowlist_var(var);
    }
    let bindings = builder.generate().map_err(|e| format!("generating the FFmpeg bindings: {e}"))?;
    bindings.write_to_file(out.join("ffmpeg.rs")).map_err(|e| format!("writing the FFmpeg bindings: {e}"))
}

/// The profile folder (where gallery.exe is) gets the DLLs, so Qt Multimedia finds them there.
fn copy_dlls_beside_app(bin: &Path) -> Result<(), String> {
    // OUT_DIR is target\<profile>\build\pg-video-<hash>\out with stable cargo, and target\<profile>\build\pg-video\<hash>\out
    // with the pinned nightly: the profile folder is the parent of the nearest `build` folder above OUT_DIR. (Counting
    // ancestors put the copies in target\<profile>\build\ under the nightly, where the binaries don't look.)
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set: cargo sets it for build scripts")?);
    let profile = out
        .ancestors()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("build"))
        .and_then(Path::parent)
        .ok_or_else(|| format!("{} has no target profile folder", out.display()))?;
    let entries = std::fs::read_dir(bin).unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
    let dlls: Vec<(PathBuf, OsString)> = entries
        .filter_map(|e| e.ok().map(|e| (e.path(), e.file_name())))
        .filter(|(_, name)| {
            let name = name.to_str().unwrap_or_default();
            name.ends_with(".dll") && DLLS.iter().any(|d| name.starts_with(&format!("{d}-")))
        })
        .collect();
    assert_eq!(
        dlls.len(),
        DLLS.len(),
        "the FFmpeg DLLs are missing from {}: FFMPEG_DIR must be an FFmpeg build with a bin folder",
        bin.display()
    );
    std::fs::create_dir_all(profile).map_err(|e| format!("creating {}: {e}", profile.display()))?;
    for (dll, name) in &dlls {
        let dest = profile.join(name);
        let same = |a: &Path, b: &Path| {
            let (a, b) = (std::fs::metadata(a), std::fs::metadata(b));
            matches!((a, b), (Ok(a), Ok(b)) if a.len() == b.len() && a.modified().ok() == b.modified().ok())
        };
        if !same(dll, &dest) {
            std::fs::copy(dll, &dest)
                .unwrap_or_else(|e| panic!("copying {} to {}: {e}", dll.display(), profile.display()));
        }
    }
    Ok(())
}
