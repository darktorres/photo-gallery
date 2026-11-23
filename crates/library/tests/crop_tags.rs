//! A crop whose copy can't be given its tags (here ExifTool can't run) is a failure, and leaves no untagged copy at its
//! output, nor a temporary beside it; a cut whose tags can't be copied leaves its original as it was. The program that
//! can't run is given to the crop (`save_with`, `cut_with`), so no test reads or sets the environment for it.

use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::path::PathArena;

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The letterboxed picture (320 × 240, black bars 40 deep, as `crop.rs`'s) written as a TIFF.
fn letterboxed_tiff() -> pg_codec::Result<Vec<u8>> {
    pg_codec::fixtures::encode(320, 240, &pg_testkit::letterboxed_rgb(320, 240, 40), pg_codec::fixtures::Encoding::Tiff)
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
fn a_copy_that_cannot_be_tagged_fails_and_leaves_nothing_behind() -> Result<()> {
    let dir = pg_testkit::run_folder_with("scratch_library", "crop-tags", &["images"]).unwrap();
    // A TIFF: the copy has no metadata of its own, so the crop copies the tags with ExifTool.
    std::fs::write(dir.join("images").join("d.tif"), letterboxed_tiff()?).unwrap();

    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let id = state::file_id(&c, "d.tif").unwrap();

    // ExifTool can't run: the copy is published, then its tags can't be copied into it.
    let missing = dir.join("no-such-exiftool");
    let out = dir.join("out");
    let stats = pg_crop::save_with(missing.as_os_str(), &c, &[id], &out, &opts, &ctl).unwrap();
    assert_eq!((stats.saved, stats.failed), (0, 1), "the copy's tags could not be written: a failure");
    let left = files_under(&out);
    assert!(left.is_empty(), "no untagged copy and no temporary is left: {left:?}");
    Ok(())
}

/// A TIFF cut whose tags can't be copied (ExifTool can't run) fails, and the original is as it was: the bytes, no
/// temporary beside it, and the catalog still has its borders to cut.
#[test]
fn a_cut_that_cannot_be_tagged_leaves_the_original_as_it_was() -> Result<()> {
    let dir = pg_testkit::run_folder_with("scratch_library", "crop-tags-cut", &["images"]).unwrap();
    let images = dir.join("images");
    let path = images.join("d.tif");
    std::fs::write(&path, letterboxed_tiff()?).unwrap();

    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let id = state::file_id(&c, "d.tif").unwrap();
    let before = std::fs::read(&path).unwrap();

    let missing = dir.join("no-such-exiftool");
    let stats = pg_crop::cut_with(missing.as_os_str(), &c, &[id], true, &opts, &ctl, &|_| true).unwrap();
    assert_eq!((stats.saved, stats.failed), (0, 1), "the tags could not be copied: a failure ({:?})", stats.failures);
    assert_eq!(std::fs::read(&path).unwrap(), before, "the original is as it was");
    assert_eq!(files_under(&images), vec![path.clone()], "no temporary is left beside it");
    assert!(
        c.crop_source(id, &mut PathArena::default()).unwrap().is_some(),
        "the catalog still has the borders to cut"
    );
    Ok(())
}
