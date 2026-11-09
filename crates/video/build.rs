//! FFmpeg's C types, constants and version numbers for pg-video, generated from its headers, so no struct layout is written
//! by hand. The headers are `FFMPEG_DIR\include` when that is set, else on Windows the pinned build in `.deps\ffmpeg`, else
//! the development packages' (pkg-config). pg-video doesn't link FFmpeg: it loads the
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
    "AVIOContext",
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
    "AVFMT_FLAG_CUSTOM_IO",
    "AVCOL_SPC_.*",
    "AVCOL_RANGE_.*",
    "LIB(AVCODEC|AVFORMAT|AVUTIL|SWRESAMPLE|SWSCALE)_VERSION_MAJOR",
];

/// The FFmpeg libraries whose headers and DLLs pg-video uses.
const LIBRARIES: &[&str] = &["libavcodec", "libavformat", "libavutil", "libswscale", "libswresample"];

/// The development packages that provide them on Debian and Ubuntu (docker/apt-packages.txt, CI's setup).
const APT_PACKAGES: &str = "libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev";

/// Where FFmpeg's headers are: the include folders the bindings are generated from, and, on Windows, the prefix whose `bin`
/// folder holds the DLLs.
struct Headers {
    include: Vec<PathBuf>,
    prefix: Option<PathBuf>,
}

/// A failure is an error (cargo prints it and the build stops) or a panic with its message.
fn main() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    // pkg-config finds the system's headers through these.
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    // CI sets PG_VIDEO_CONTENT_KEY to a hash of the FFmpeg packages it installs (ci.yml). It installs them fresh on every
    // run, so the headers' mtimes are new every run; with the key, the build reruns on the key and not on the folder.
    println!("cargo:rerun-if-env-changed=PG_VIDEO_CONTENT_KEY");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let headers = headers(windows)?;
    // Local builds watch the library folders (a header change reruns the bindings). With the key set, the key is the trigger.
    if !has_content_key() {
        for include in &headers.include {
            for lib in LIBRARIES {
                let folder = include.join(lib);
                if folder.is_dir() {
                    println!("cargo:rerun-if-changed={}", folder.display());
                }
            }
        }
    }
    generate_bindings(&headers.include)?;

    if windows {
        let prefix = headers.prefix.ok_or("the Windows build has no FFmpeg prefix with its DLLs")?;
        let bin = prefix.join("bin");
        // Compiled in, so the loader can find the DLLs when no copy is beside the executable.
        println!("cargo:rustc-env=PG_FFMPEG_BIN={}", bin.display());
        println!("cargo:rerun-if-changed={}", bin.display());
        copy_dlls_beside_app(&bin)?;
    }
    Ok(())
}

/// The headers of the FFmpeg this build uses, in order of preference: `FFMPEG_DIR` (a prefix with include/ and bin/, on any
/// OS); on Windows, the pinned build `just setup` fetches into the repository's `.deps/ffmpeg`; elsewhere, the development
/// packages' headers, as pkg-config gives them.
fn headers(windows: bool) -> Result<Headers, String> {
    let prefix = match std::env::var_os("FFMPEG_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if windows => Some(workspace_root()?.join(".deps").join("ffmpeg")),
        None => None,
    };
    let Some(prefix) = prefix else {
        return Ok(Headers { include: pkg_config_includes()?, prefix: None });
    };
    let include = prefix.join("include");
    if !include.join("libavcodec").join("avcodec.h").is_file() {
        return Err(format!(
            "no FFmpeg headers in {}: run `just setup` to fetch the pinned build (Windows), or set FFMPEG_DIR in .env to an \
             FFmpeg 9.0 shared build",
            prefix.display()
        ));
    }
    Ok(Headers { include: vec![include], prefix: Some(prefix) })
}

/// The repository's root: pg-video is at crates/video, two folders below it.
fn workspace_root() -> Result<PathBuf, String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or("CARGO_MANIFEST_DIR is not set: cargo sets it for build scripts")?;
    Path::new(&manifest)
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no repository folder above it", Path::new(&manifest).display()))
}

/// The include folders of the FFmpeg development packages, as `pkg-config --cflags-only-I` gives them.
fn pkg_config_includes() -> Result<Vec<PathBuf>, String> {
    let hint = format!("install pkg-config and the FFmpeg development packages ({APT_PACKAGES}), or set FFMPEG_DIR");
    let output = std::process::Command::new("pkg-config")
        .arg("--cflags-only-I")
        .args(LIBRARIES)
        .output()
        .map_err(|e| format!("pkg-config can't run ({e}): {hint}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pkg-config finds no FFmpeg libraries ({}): {hint}", stderr.trim()));
    }
    let mut include: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|flag| flag.strip_prefix("-I"))
        .map(PathBuf::from)
        .collect();
    include.dedup();
    if include.is_empty() {
        return Err(format!("pkg-config gives no include folder for FFmpeg: {hint}"));
    }
    Ok(include)
}

/// `OUT_DIR/ffmpeg.rs`: the types and constants from the headers, found in the include folders.
fn generate_bindings(include: &[PathBuf]) -> Result<(), String> {
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
    let mut builder = bindgen::Builder::default().derive_default(false).layout_tests(false).generate_comments(false);
    for folder in include {
        builder = builder.clang_arg(format!("-I{}", folder.display()));
    }
    for header in headers {
        let found = include
            .iter()
            .map(|folder| folder.join(header))
            .find(|path| path.is_file())
            .ok_or_else(|| format!("no FFmpeg header {header} in {}", join_paths(include)))?;
        builder = builder.header(found.to_string_lossy());
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

/// The folders as one list, for a message.
fn join_paths(folders: &[PathBuf]) -> String {
    folders.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", ")
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

/// Whether the content key is set (non-empty), as CI sets it: the build then doesn't watch the include folder.
fn has_content_key() -> bool {
    std::env::var("PG_VIDEO_CONTENT_KEY").is_ok_and(|key| !key.is_empty())
}
