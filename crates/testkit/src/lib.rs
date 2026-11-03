//! Helpers for the crates' tests. Tests write their files under `PG_TEST_DIR` when it is set, and under the default
//! folder each test names otherwise, so a test never writes to the system drive by accident.

use std::path::PathBuf;

/// The base folder for a test's files: `PG_TEST_DIR`, or `default`.
fn base(default: &str) -> PathBuf {
    std::env::var_os("PG_TEST_DIR").map_or_else(|| PathBuf::from(default), PathBuf::from)
}

/// A fresh folder for one test: `name-<process id>` under the base, emptied if it exists.
pub fn test_folder(default: &str, name: &str) -> PathBuf {
    let dir = base(default).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fresh folder for a test binary's run: `prefix-<process id>` under the base. The folders `prefix-*` that earlier
/// runs left are removed first, so the base keeps one run's folder per prefix.
pub fn run_folder(default: &str, prefix: &str) -> PathBuf {
    let base = base(default);
    let earlier = format!("{prefix}-");
    for old in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if old.file_name().to_string_lossy().starts_with(&earlier) {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let dir = base.join(format!("{earlier}{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// [`run_folder`], with the folders `dirs` (paths relative to it, made as needed) made in it.
pub fn run_folder_with(default: &str, prefix: &str, dirs: &[&str]) -> PathBuf {
    let dir = run_folder(default, prefix);
    for sub in dirs {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    dir
}

/// An animated GIF of `frames` 4 × 4 frames, the first red and each after it brighter.
pub fn gif(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
    for i in 0..frames {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([i as u8 * 60, 0, 0, 255]));
        enc.encode_frame(image::Frame::new(img)).unwrap();
    }
    drop(enc);
    out
}

/// An animated PNG of `frames` 2 × 2 frames of one grey, or a still PNG for one frame.
pub fn apng(frames: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 2);
    enc.set_color(png::ColorType::Rgb);
    if frames > 1 {
        enc.set_animated(frames, 0).unwrap();
    }
    let mut w = enc.write_header().unwrap();
    for _ in 0..frames {
        w.write_image_data(&[7; 12]).unwrap();
    }
    w.finish().unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_folder_is_fresh_and_a_run_folder_replaces_earlier_runs() {
        let default = std::env::temp_dir().join("pg-testkit-check");
        let default = default.to_string_lossy().into_owned();
        // Unless PG_TEST_DIR is set, which the default covers: then the folders are made under it instead.
        let first = test_folder(&default, "fresh");
        std::fs::write(first.join("left-over.txt"), b"x").unwrap();
        let again = test_folder(&default, "fresh");
        assert_eq!(first, again);
        assert_eq!(std::fs::read_dir(&again).unwrap().count(), 0, "emptied");

        let earlier = base(&default).join("run-999999999");
        std::fs::create_dir_all(&earlier).unwrap();
        let run = run_folder(&default, "run");
        assert!(run.is_dir());
        assert!(!earlier.exists(), "an earlier run's folder is removed");
    }
}
