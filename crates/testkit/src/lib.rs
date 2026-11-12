//! Helpers for the crates' tests. Tests write their files under `PG_TEST_DIR` when it is set, and under the temp folder
//! otherwise, in the folder each test names. Set `PG_TEST_DIR` to keep the files off the system drive.
//!
//! The folders and their protocol are in pg-platform (`testing`): this crate re-exports them under the names the tests
//! use, and adds the image fixtures.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

pub use pg_platform::testing::{held, missing_tool, odd_name, skip};

/// A fresh folder for one test (or one part of a test) in a test binary's run: `prefix-<process id>-<n>` under the base
/// (`PG_TEST_DIR`, else the temp folder's `scratch`). The run folders of `prefix` that no running process holds are
/// removed first, and the folder is owned by its lock from the moment it has a name (see `pg_platform::testing`).
pub fn run_folder(scratch: &str, prefix: &str) -> Result<PathBuf> {
    pg_platform::testing::run_folder(scratch, prefix)
        .with_context(|| format!("make the run folder {prefix} in {scratch}"))
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
    fn a_run_folder_is_fresh_and_each_call_has_its_own() {
        let first = run_folder("pg-testkit-check", "fresh").unwrap();
        std::fs::write(first.join("kept.txt"), b"x").unwrap();
        let second = run_folder("pg-testkit-check", "fresh").unwrap();
        assert_ne!(first, second);
        assert!(first.join("kept.txt").is_file(), "a call does not remove another call's folder");
        assert_eq!(std::fs::read_dir(&second).unwrap().count(), 0, "a new folder is empty");
    }
}
