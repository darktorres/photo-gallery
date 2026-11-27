//! Cropped copies with a folder whose name ends in a dot (Windows drops it from the plain path): the source is read
//! from that folder and the copy written into it, each by its disk path, and a sibling folder without the dot (the one
//! the plain path names) is neither read nor written. Windows only: the names are normalized there alone.
#![cfg(windows)]

use std::path::Path;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::ids::FileId;
use pg_platform::native::verbatim;
use pg_testkit::files_under;

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A letterboxed JPEG (the testkit's picture) with `pixel` at its centre, so that two files' bytes differ.
fn jpeg(pixel: [u8; 3]) -> Result<Vec<u8>> {
    let mut rgb = pg_testkit::letterboxed_rgb(320, 240, 40);
    let at = (120 * 320 + 160) * 3;
    rgb[at..at + 3].copy_from_slice(&pixel);
    pg_codec::fixtures::encode(320, 240, &rgb, pg_codec::fixtures::Encoding::Jpeg).map_err(Into::into)
}

/// A library at `dir\images`, with two folders: `Acme Inc.` (its name ends in a dot) and its sibling `Acme Inc`, each
/// with an `a.jpg`. Scanned, imported and with the borders found. Returns the catalog, the dotted folder's file and the
/// sibling's.
fn library(dir: &Path) -> Result<(Catalog, FileId, FileId)> {
    let images = dir.join("images");
    let (dotted, sibling) = (images.join("Acme Inc."), images.join("Acme Inc"));
    for (folder, pixel) in [(&dotted, [255u8, 0, 0]), (&sibling, [0u8, 0, 255])] {
        std::fs::create_dir_all(verbatim(folder))?;
        std::fs::write(verbatim(&folder.join("a.jpg")), jpeg(pixel)?)?;
    }
    let mut c = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES)?;
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let root = pg_platform::names::normalize_path(&images)?;
    c.add_root(&root)?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl)?;
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl)?;
    let files = state::present_files(&c)?;
    if files.len() != 2 {
        return Err(format!("two files were expected: {files:?}").into());
    }
    let (mut dotted_id, mut sibling_id) = (None, None);
    for (id, _) in &files {
        let path = c.file_path(*id)?.ok_or("a file without a path")?;
        let in_dotted = path.parts.iter().rev().nth(1).is_some_and(|folder| folder.display().ends_with('.'));
        if in_dotted {
            dotted_id = Some(*id);
        } else {
            sibling_id = Some(*id);
        }
    }
    Ok((c, dotted_id.ok_or("the file in the dotted folder")?, sibling_id.ok_or("the sibling's file")?))
}

/// The name of the folder `path` is in.
fn folder_name(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A source whose folder ends in a dot is copied from that folder, beside its sibling's copy, and both originals are
/// left as they were. Nothing else is left behind.
#[test]
fn a_source_in_a_folder_ending_in_a_dot_is_copied_from_that_folder() -> Result<()> {
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_library", "folders-source")?)?;
    let (c, dotted, sibling) = library(&dir)?;
    let (dotted_original, sibling_original) = (
        std::fs::read(verbatim(&dir.join("images").join("Acme Inc.").join("a.jpg")))?,
        std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?,
    );
    let out = dir.join("out");
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = pg_crop::save(&c, &[dotted, sibling], &out, &opts, &Control::new())?;

    assert_eq!((stats.saved, stats.failed), (2, 0), "{:?}", stats.failures);
    assert_eq!(std::fs::read(verbatim(&dir.join("images").join("Acme Inc.").join("a.jpg")))?, dotted_original);
    assert_eq!(std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?, sibling_original);
    let copies = files_under(&out);
    assert_eq!(copies.len(), 2, "a copy of each: {copies:?}");
    let mut folders: Vec<String> = copies.iter().map(|p| folder_name(p)).collect();
    folders.sort();
    assert_eq!(folders, ["Acme Inc", "Acme Inc."], "each copy in its own folder: {copies:?}");
    // Each copy is of its own source: the two sources' pixels differ, so their copies do.
    let [first, second] = copies.as_slice() else { return Err(format!("two copies: {copies:?}").into()) };
    assert_ne!(std::fs::read(verbatim(first))?, std::fs::read(verbatim(second))?, "{copies:?}");
    drop(c);
    let _ = std::fs::remove_dir_all(verbatim(&dir));
    Ok(())
}

/// A copy into a folder whose name ends in a dot is written into that folder, and no folder without the dot is made.
#[test]
fn a_copy_into_a_folder_ending_in_a_dot_is_written_there() -> Result<()> {
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_library", "folders-copy")?)?;
    let (c, _dotted, sibling) = library(&dir)?;
    let original = std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?;
    let out = dir.join("Out.");
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = pg_crop::save(&c, &[sibling], &out, &opts, &Control::new())?;

    assert_eq!((stats.saved, stats.failed), (1, 0), "{:?}", stats.failures);
    assert_eq!(files_under(&verbatim(&out)).len(), 1, "the copy is in Out.");
    assert!(!dir.join("Out").exists(), "a folder without the dot was made");
    assert_eq!(std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?, original);
    drop(c);
    let _ = std::fs::remove_dir_all(verbatim(&dir));
    Ok(())
}
