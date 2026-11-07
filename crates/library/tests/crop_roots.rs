//! Cropped copies of two library folders that have the same last name: each copy is kept, none overwritten.

use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_library::{borders, crop};
use pg_model::ids::FileId;
use pg_model::rules::PathRules;

/// A letterboxed picture (black bars at the top and bottom, the content between them).
fn letterboxed() -> image::RgbImage {
    image::RgbImage::from_fn(320, 240, |x, y| {
        if !(40..200).contains(&y) {
            image::Rgb([0, 0, 0])
        } else {
            let byte = |v: u32| u8::try_from(v % 256).unwrap_or(u8::MAX);
            image::Rgb([byte(x * 7), byte(y * 13), byte((x + y) * 3)])
        }
    })
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn library_folders_with_one_name_keep_their_copies_apart() {
    let dir = pg_testkit::run_folder("scratch_rate", "crop-roots").unwrap();
    let mut c = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);

    // Two folders, both called "images", with different pictures (the same bars, so both are cut the same way).
    for (sub, pixel) in [("one", [255, 0, 0]), ("two", [0, 0, 255])] {
        let folder = dir.join(sub).join("images");
        std::fs::create_dir_all(&folder).unwrap();
        let mut img = letterboxed();
        img.put_pixel(160, 120, image::Rgb(pixel));
        img.save(folder.join("a.png")).unwrap();
        let root = pg_scan::normalize_path(&folder).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
    }
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    borders::find(&mut c, &opts, &ctl).unwrap();

    let ids: Vec<FileId> = state::present_files(&c).unwrap().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);

    let out = dir.join("out");
    let s = crop::save(&c, &ids, &out, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.failed), (2, 0), "{:?}", s.error);
    assert_eq!(files_under(&out).len(), 2, "both copies are kept: {:?}", files_under(&out));
}
