//! Puts the FFmpeg DLLs (`$FFMPEG_DIR\bin`, set in `.cargo/config.toml`) next to the binaries: gallery.exe and
//! the test executables load them at startup. Qt Multimedia plays videos with the same DLLs (same major version),
//! so the app is deployed with `windeployqt --no-ffmpeg` (crates/README.md "Build").

use std::path::{Path, PathBuf};

/// What pg-video and Qt Multimedia load; avdevice and avfilter are used by neither.
const DLLS: &[&str] = &["avcodec", "avformat", "avutil", "swresample", "swscale"];

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    // Only Windows loads DLLs from beside the binary. Elsewhere (the callgrind benches under WSL2) FFmpeg is the
    // system's, and FFMPEG_DIR points at its headers.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Without it the app would start and fail at once for want of the DLLs, so the build stops here instead.
    let dir = std::env::var_os("FFMPEG_DIR").unwrap_or_else(|| {
        panic!(
            "FFMPEG_DIR is not set: the FFmpeg DLLs can't be copied beside the binaries (.cargo/config.toml sets it)"
        )
    });
    let bin = Path::new(&dir).join("bin");
    println!("cargo:rerun-if-changed={}", bin.display());
    // OUT_DIR is target\<profile>\build\pg-video-<hash>\out.
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let profile = out.ancestors().nth(3).expect("target profile folder");
    let entries = std::fs::read_dir(&bin).unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
    let dlls: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            name.ends_with(".dll") && DLLS.iter().any(|d| name.starts_with(&format!("{d}-")))
        })
        .collect();
    assert_eq!(
        dlls.len(),
        DLLS.len(),
        "the FFmpeg DLLs are missing from {}: FFMPEG_DIR must be an FFmpeg build with a bin folder",
        bin.display()
    );
    for to in [profile.to_path_buf(), profile.join("deps")] {
        std::fs::create_dir_all(&to).unwrap();
        for dll in &dlls {
            let dest = to.join(dll.file_name().unwrap());
            let same = |a: &Path, b: &Path| {
                let (a, b) = (std::fs::metadata(a), std::fs::metadata(b));
                matches!((a, b), (Ok(a), Ok(b)) if a.len() == b.len() && a.modified().ok() == b.modified().ok())
            };
            if !same(dll, &dest) {
                std::fs::copy(dll, &dest)
                    .unwrap_or_else(|e| panic!("copying {} to {}: {e}", dll.display(), to.display()));
            }
        }
    }
}
