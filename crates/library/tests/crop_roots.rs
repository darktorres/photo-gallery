//! Cropped copies of two library folders that have the same last name: each copy is kept, none overwritten.

use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::ids::FileId;

/// A letterboxed picture, 320 × 240 as RGB bytes (black bars at the top and bottom, the content between them).
fn letterboxed() -> Vec<u8> {
    let byte = |v: u32| u8::try_from(v % 256).unwrap_or(u8::MAX);
    (0..240u32)
        .flat_map(|y| {
            (0..320u32).flat_map(move |x| {
                if !(40..200).contains(&y) { [0, 0, 0] } else { [byte(x * 7), byte(y * 13), byte((x + y) * 3)] }
            })
        })
        .collect()
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
    let dir = pg_testkit::run_folder("scratch_library", "crop-roots").unwrap();
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);

    // Two folders, both called "images", with different pictures (the same bars, so both are cut the same way).
    for (sub, pixel) in [("one", [255u8, 0, 0]), ("two", [0u8, 0, 255])] {
        let folder = dir.join(sub).join("images");
        std::fs::create_dir_all(&folder).unwrap();
        let mut img = letterboxed();
        let at = (120 * 320 + 160) * 3;
        img[at..at + 3].copy_from_slice(&pixel);
        std::fs::write(folder.join("a.png"), pg_testkit::encode(320, 240, &img, pg_testkit::Encoding::Png).unwrap())
            .unwrap();
        let root = pg_platform::names::normalize_path(&folder).unwrap();
        c.add_root(&root).unwrap();
        pg_scan::scan(&mut c, &root, &ctl).unwrap();
    }
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();

    let ids: Vec<FileId> = state::present_files(&c).unwrap().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);

    let out = dir.join("out");
    let s = pg_crop::save(&c, &ids, &out, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.failed), (2, 0), "{:?}", s.failures);
    assert_eq!(files_under(&out).len(), 2, "both copies are kept: {:?}", files_under(&out));
}
