//! FFmpeg's C types, constants and version numbers for pg-video, generated from its headers, so no struct layout is written
//! by hand. The headers are `FFMPEG_DIR\include` when that is set, else on Windows the pinned build in `.deps\ffmpeg`, else
//! the development packages' (pkg-config). pg-video doesn't link FFmpeg: it loads the
//! DLLs at run time (src/ffi.rs), and the functions it calls are looked up by name there.
//!
//! On Windows the FFmpeg DLLs are also copied beside the app's binary: Qt Multimedia loads the same DLLs to play
//! videos (same major version), so gallery.exe needs them beside it. The test executables
//! don't: pg-video finds them by their full path.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What Qt Multimedia loads; avdevice and avfilter are used by neither.
const DLLS: &[&str] = &["avcodec", "avformat", "avutil", "swresample", "swscale"];

/// The C types pg-video uses. Enums come out as constants, so their values are the C ones. Bindgen also emits the types
/// they reach (`AVPacketSideData` is one of those, a field type of two structs, and no source names it).
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
    "AVRational",
    "AVStream",
    "SwsContext",
];

/// The C constants pg-video uses, by exact name: the defines, and the enum members (bindgen emits those under their
/// enum's name, as `AVMediaType_AVMEDIA_TYPE_VIDEO`).
const CONSTS: &[&str] =
    &["AV_LOG_FATAL", "AV_PKT_DATA_DISPLAYMATRIX", "AVMEDIA_TYPE_VIDEO", "AV_PIX_FMT_RGB24", "AVFMT_FLAG_CUSTOM_IO"];

/// The enum members pg-video uses by prefix: the colour space and range enums.
const PREFIXES: &[&str] = &["AVCOL_SPC_", "AVCOL_RANGE_"];

/// The libraries whose major version pg-video reads (`crates/video/src/ffi/load.rs`), as `LIB<name>_VERSION_MAJOR`.
const MAJORS: &[&str] = &["AVCODEC", "AVFORMAT", "AVUTIL", "SWSCALE"];

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

/// A failure is an error: `main` returns it, and cargo prints it and the build stops.
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

    // The static build (PG_FFMPEG_STATIC, the static FFmpeg's lib folder): FFmpeg is linked
    // in, so there are no DLLs to copy or load. ffi/load.rs then points its functions at the linked ones (cfg pg_ffmpeg_static).
    println!("cargo:rerun-if-env-changed=PG_FFMPEG_STATIC");
    println!("cargo:rustc-check-cfg=cfg(pg_ffmpeg_static)");
    if let Some(lib_dir) = std::env::var_os("PG_FFMPEG_STATIC") {
        if !windows {
            return Err("PG_FFMPEG_STATIC is for the Windows static build only".into());
        }
        println!("cargo:rustc-cfg=pg_ffmpeg_static");
        link_static(Path::new(&lib_dir));
        return Ok(());
    }

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
            "no FFmpeg headers in {}: run `just setup` to fetch the pinned build (Windows), or set FFMPEG_DIR in .env to a \
             shared build of the release the Justfile's ffmpeg_url pins",
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

/// The static FFmpeg's libraries (MSVC, built by tools/build_static_ffmpeg.py) from `lib_dir`, linked into pg-video.
/// Its link needs these Windows libraries: FFmpeg's .pc files don't list them.
fn link_static(lib_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    for lib in ["avcodec", "avformat", "avutil", "swresample", "swscale"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    for lib in ["user32", "bcrypt"] {
        println!("cargo:rustc-link-lib={lib}");
    }
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
    for name in CONSTS {
        builder = builder.allowlist_var(format!("^{name}$"));
    }
    for prefix in PREFIXES {
        builder = builder.allowlist_var(format!("^{prefix}.*"));
    }
    for lib in MAJORS {
        builder = builder.allowlist_var(format!("^LIB{lib}_VERSION_MAJOR$"));
    }
    let bindings = builder.generate().map_err(|e| format!("generating the FFmpeg bindings: {e}"))?;
    // Formatted once here (the bindings' `Display` runs rustfmt), then checked and written as they are.
    let text = bindings.to_string();
    let src = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?).join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    check_bindings(&text, &src)?;
    std::fs::write(out.join("ffmpeg.rs"), &text).map_err(|e| format!("writing the FFmpeg bindings: {e}"))
}

/// Checks the generated bindings against the lists above, and the lists against pg-video's source. Bindgen emits nothing
/// for a name that matches no symbol, and a name no source uses is dead weight, so either would otherwise pass silently.
/// The names are read as identifiers, not as substrings: `AVCodec` is a prefix of `AVCodecContext`.
fn check_bindings(text: &str, src: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    let structs = declared_names(text, "pub struct ");
    let aliases = declared_names(text, "pub type ");
    let consts = declared_names(text, "pub const ");
    let source = source_text(src)?;
    for ty in TYPES {
        if !structs.contains(ty) && !aliases.contains(ty) {
            problems.push(format!("TYPES: {ty} is not declared in the bindings"));
        }
        if !occurs(&source, ty, true) {
            problems.push(format!("TYPES: pg-video's source names no {ty}: remove it"));
        }
    }
    let mut names: Vec<String> = CONSTS.iter().map(|name| name.to_string()).collect();
    names.extend(MAJORS.iter().map(|lib| format!("LIB{lib}_VERSION_MAJOR")));
    for name in &names {
        // A define is declared under its own name, an enum member under its enum's name (`X_<name>`).
        if !consts.iter().any(|c| c == name || c.ends_with(&format!("_{name}"))) {
            problems.push(format!("CONSTS: {name} is not declared in the bindings"));
        }
        if !occurs(&source, name, true) {
            problems.push(format!("CONSTS: pg-video's source names no {name}: remove it"));
        }
    }
    for prefix in PREFIXES {
        if !consts.iter().any(|c| c.starts_with(prefix) || c.contains(&format!("_{prefix}"))) {
            problems.push(format!("PREFIXES: no constant in the bindings starts with {prefix}"));
        }
        if !occurs(&source, prefix, false) {
            problems.push(format!("PREFIXES: pg-video's source names no {prefix}: remove it"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("the FFmpeg bindings do not match build.rs: {}", problems.join("; ")))
    }
}

/// The identifiers that follow `keyword` at the start of a line of `text` (`pub struct AVFrame {` gives `AVFrame`).
fn declared_names<'a>(text: &'a str, keyword: &str) -> Vec<&'a str> {
    text.lines().filter_map(|line| line.trim_start().strip_prefix(keyword)).map(identifier).collect()
}

/// The identifier at the start of `text`.
fn identifier(text: &str) -> &str {
    let end = text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(text.len());
    // `find` returns a character boundary, so `get` is `Some`.
    text.get(..end).unwrap_or(text)
}

/// Whether `name` occurs in `text` as an identifier, or as the tail of one after an underscore (the enum member
/// `AVMediaType_AVMEDIA_TYPE_VIDEO` names `AVMEDIA_TYPE_VIDEO`). With `whole`, it must also end at an identifier's end;
/// a prefix (`AVCOL_SPC_`) ends where its members' names continue.
fn occurs(text: &str, name: &str, whole: bool) -> bool {
    text.match_indices(name).any(|(at, _)| {
        // Each index is a match's start or end, so it is a character boundary and `get` is `Some`.
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let after = text.get(at + name.len()..).and_then(|tail| tail.chars().next());
        let starts = !before.is_some_and(|c| c.is_ascii_alphanumeric());
        let ends = !whole || !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        starts && ends
    })
}

/// Every `.rs` file under `dir`, concatenated.
fn source_text(dir: &Path) -> Result<String, String> {
    let mut text = String::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("reading {}: {e}", dir.display()))?.path();
        if path.is_dir() {
            text.push_str(&source_text(&path)?);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            text.push_str(&std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?);
        }
    }
    Ok(text)
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
    let entries = std::fs::read_dir(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let dlls: Vec<(PathBuf, OsString)> = entries
        .filter_map(|e| e.ok().map(|e| (e.path(), e.file_name())))
        .filter(|(_, name)| {
            let name = name.to_str().unwrap_or_default();
            name.ends_with(".dll") && DLLS.iter().any(|d| name.starts_with(&format!("{d}-")))
        })
        .collect();
    if dlls.len() != DLLS.len() {
        return Err(format!(
            "the FFmpeg DLLs are missing from {}: FFMPEG_DIR must be an FFmpeg build with a bin folder",
            bin.display()
        ));
    }
    std::fs::create_dir_all(profile).map_err(|e| format!("creating {}: {e}", profile.display()))?;
    for (dll, name) in &dlls {
        let dest = profile.join(name);
        let same = |a: &Path, b: &Path| {
            let (a, b) = (std::fs::metadata(a), std::fs::metadata(b));
            matches!((a, b), (Ok(a), Ok(b)) if a.len() == b.len() && a.modified().ok() == b.modified().ok())
        };
        if !same(dll, &dest) {
            std::fs::copy(dll, &dest)
                .map_err(|e| format!("copying {} to {}: {e}", dll.display(), profile.display()))?;
        }
    }
    Ok(())
}

/// Whether the content key is set (non-empty), as CI sets it: the build then doesn't watch the include folder.
fn has_content_key() -> bool {
    std::env::var("PG_VIDEO_CONTENT_KEY").is_ok_and(|key| !key.is_empty())
}
