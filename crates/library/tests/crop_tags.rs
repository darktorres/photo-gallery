//! A crop whose copy can't be given its tags (here ExifTool can't run) is a failure, and leaves no untagged copy at its
//! output, nor a temporary beside it. In its own test binary: it sets `PG_EXIFTOOL` for the whole process, which the
//! other crop tests must not see.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;

const W: u32 = 320;
const H: u32 = 240;
/// The letterbox: black bars this deep at the top and bottom, photo-like content between them (as `crop.rs`'s).
const BAR: u32 = 40;

fn letterboxed() -> image::RgbImage {
    let hash = |a: u32, b: u32| (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
    image::RgbImage::from_fn(W, H, |x, y| {
        if !(BAR..H - BAR).contains(&y) {
            return image::Rgb([0, 0, 0]);
        }
        let block = hash(x / 6, y / 6);
        let noise = hash(x, y) % 24;
        let c = |shift: u32| u8::try_from(60 + (block >> shift) % 150 + noise).unwrap_or(u8::MAX);
        image::Rgb([c(0), c(8), c(16)])
    })
}

/// Every file under `dir`, at any depth.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[test]
fn a_copy_that_cannot_be_tagged_fails_and_leaves_nothing_behind() {
    let dir = pg_testkit::run_folder_with("scratch_rate", "crop_tags", &["images"]).unwrap();
    // A TIFF: the copy has no metadata of its own, so the crop copies the tags with ExifTool.
    let mut bytes = Cursor::new(Vec::new());
    letterboxed().write_to(&mut bytes, image::ImageFormat::Tiff).unwrap();
    std::fs::write(dir.join("images").join("d.tif"), bytes.get_ref()).unwrap();

    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &ctl).unwrap();
    let id = state::file_id(&c, "d.tif").unwrap();

    // ExifTool can't run: the copy is published, then its tags can't be copied into it.
    // SAFETY: this test binary runs no other test that reads PG_EXIFTOOL, so nothing reads it concurrently.
    unsafe { std::env::set_var("PG_EXIFTOOL", dir.join("no-such-exiftool")) };
    let out = dir.join("out");
    let stats = pg_crop::save(&c, &[id], &out, &opts, &ctl).unwrap();
    assert_eq!((stats.saved, stats.failed), (0, 1), "the copy's tags could not be written: a failure");
    let left = files_under(&out);
    assert!(left.is_empty(), "no untagged copy and no temporary is left: {left:?}");
}
