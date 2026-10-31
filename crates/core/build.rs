//! Puts the FFmpeg DLLs (`$FFMPEG_DIR\bin`, set in `.cargo/config.toml`) next to the binaries: gallery.exe and
//! the test executables load them at startup. Qt Multimedia plays videos with the same DLLs (same major version),
//! so the app is deployed with `windeployqt --no-ffmpeg` (crates/README.md "Build").

use std::path::{Path, PathBuf};

/// What pg-core and Qt Multimedia load; avdevice and avfilter are used by neither.
const DLLS: &[&str] = &["avcodec", "avformat", "avutil", "swresample", "swscale"];

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Some(dir) = std::env::var_os("FFMPEG_DIR") else { return };
    let bin = Path::new(&dir).join("bin");
    println!("cargo:rerun-if-changed={}", bin.display());
    // OUT_DIR is target\<profile>\build\pg-core-<hash>\out.
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
    assert_eq!(dlls.len(), DLLS.len(), "FFmpeg DLLs missing from {}", bin.display());
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
