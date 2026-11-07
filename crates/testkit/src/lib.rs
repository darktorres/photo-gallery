//! Helpers for the crates' tests. Tests write their files under `PG_TEST_DIR` when it is set, and under the temp folder
//! otherwise, in the folder each test names. Set `PG_TEST_DIR` to keep the files off the system drive.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

pub mod events;
pub use pg_catalog::fixture;

/// A test needs an external tool that is not installed: the test returns after this call. Without `PG_REQUIRE_TOOLS`
/// it prints that it skipped and passes; with it (CI sets it) the test fails, so a missing tool can't pass silently.
#[expect(clippy::print_stderr, reason = "a skipped test says so on stderr, where the test runner prints it")]
pub fn missing_tool(tool: &str) {
    assert!(std::env::var_os("PG_REQUIRE_TOOLS").is_none(), "{tool} not found, and PG_REQUIRE_TOOLS is set");
    eprintln!("{tool} not found: skipped");
}

/// The base folder for a test's files: `PG_TEST_DIR` when it is set, else the temp folder's `scratch` folder. The name
/// is a plain folder name (`"scratch_rate"`): no test writes to a drive it was not told about.
fn base(scratch: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("PG_TEST_DIR") {
        // `.env` writes paths with forward slashes, and the verbatim (`\\?\`) paths that pg-platform opens are not
        // normalized: a test's folders need the native separator, as the paths the app itself is given have.
        return PathBuf::from(dir.to_string_lossy().replace('/', std::path::MAIN_SEPARATOR_STR));
    }
    std::env::temp_dir().join(scratch)
}

/// A fresh folder for one test: `name-<process id>` under the base, emptied if it exists. Fails if it can't be made.
pub fn test_folder(scratch: &str, name: &str) -> Result<PathBuf> {
    let dir = base(scratch).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("make the test folder {}", dir.display()))?;
    Ok(dir)
}

/// A fresh folder for a test binary's run: `prefix-<process id>` under the base. The folders `prefix-*` that earlier
/// runs left are removed first, so the base keeps one run's folder per prefix. Fails if it can't be made.
pub fn run_folder(scratch: &str, prefix: &str) -> Result<PathBuf> {
    let base = base(scratch);
    let earlier = format!("{prefix}-");
    for old in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if old.file_name().to_string_lossy().starts_with(&earlier) {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let dir = base.join(format!("{earlier}{}", std::process::id()));
    std::fs::create_dir_all(&dir).with_context(|| format!("make the run folder {}", dir.display()))?;
    Ok(dir)
}

/// [`run_folder`], with the folders `dirs` (paths relative to it, made as needed) made in it.
pub fn run_folder_with(scratch: &str, prefix: &str, dirs: &[&str]) -> Result<PathBuf> {
    let dir = run_folder(scratch, prefix)?;
    for sub in dirs {
        let sub = dir.join(sub);
        std::fs::create_dir_all(&sub).with_context(|| format!("make the folder {}", sub.display()))?;
    }
    Ok(dir)
}

/// An animated GIF of `frames` 4 × 4 frames, the first red and each after it brighter. The red level steps by 60, so
/// at most 5 frames (0 to 240): more is an error.
pub fn gif(frames: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
    for i in 0..frames {
        let red = i
            .checked_mul(60)
            .and_then(|level| u8::try_from(level).ok())
            .ok_or_else(|| anyhow!("frame {i} of {frames}: a test GIF has at most 5 frames"))?;
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([red, 0, 0, 255]));
        enc.encode_frame(image::Frame::new(img)).with_context(|| format!("encode GIF frame {i}"))?;
    }
    drop(enc);
    Ok(out)
}

/// `rgba` (`w` × `h` pixels, four bytes each) as a PNG, with its transparency.
pub fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| anyhow!("{} bytes are not {w} × {h} RGBA pixels", rgba.len()))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, image::ImageFormat::Png).context("encode the PNG")?;
    Ok(file.into_inner())
}

/// `rgb` (`w` × `h` pixels, three bytes each) as a BMP.
pub fn bmp_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Bmp)
}

/// `rgb` as a GIF, one frame.
pub fn gif_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Gif)
}

/// `rgb` as a PNG.
pub fn png_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Png)
}

/// `rgb` as a classic TIFF (`image` writes no BigTIFF: the codec's fixtures do).
pub fn tiff_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Tiff)
}

fn still(w: u32, h: u32, rgb: &[u8], format: image::ImageFormat) -> Result<Vec<u8>> {
    let img = image::RgbImage::from_raw(w, h, rgb.to_vec())
        .ok_or_else(|| anyhow!("{} bytes are not {w} × {h} RGB pixels", rgb.len()))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, format).with_context(|| format!("encode the {format:?}"))?;
    Ok(file.into_inner())
}

/// An animated PNG of `frames` 2 × 2 frames of one grey, or a still PNG for one frame.
pub fn apng(frames: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 2);
    enc.set_color(png::ColorType::Rgb);
    if frames > 1 {
        enc.set_animated(frames, 0).context("make the PNG animated")?;
    }
    let mut w = enc.write_header().context("write the PNG header")?;
    for _ in 0..frames {
        w.write_image_data(&[7; 12]).context("write a PNG frame")?;
    }
    w.finish().context("finish the PNG")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_folder_is_fresh_and_a_run_folder_replaces_earlier_runs() {
        let scratch = "pg-testkit-check";
        // Unless PG_TEST_DIR is set, which the default covers: then the folders are made under it instead.
        let first = test_folder(scratch, "fresh").unwrap();
        std::fs::write(first.join("left-over.txt"), b"x").unwrap();
        let again = test_folder(scratch, "fresh").unwrap();
        assert_eq!(first, again);
        assert_eq!(std::fs::read_dir(&again).unwrap().count(), 0, "emptied");

        let earlier = base(scratch).join("run-999999999");
        std::fs::create_dir_all(&earlier).unwrap();
        let run = run_folder(scratch, "run").unwrap();
        assert!(run.is_dir());
        assert!(!earlier.exists(), "an earlier run's folder is removed");
    }
}
