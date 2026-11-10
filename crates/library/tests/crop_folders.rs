//! Cropped copies with a folder whose name ends in a dot or a space (Windows may drop it from the plain path ExifTool is
//! given): the copy is refused, typed, before anything is read or written; nothing is left in the copies' folder; a
//! sibling folder's file is untouched. Windows only: the names are normalized there alone.
#![cfg(windows)]

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_crop::CropError;
use pg_model::ids::FileId;
use pg_model::rules::PathRules;
use pg_platform::native::verbatim;

/// A letterboxed picture; `seed` changes its content, so that two files' bytes differ.
fn jpeg(seed: u32) -> Result<Vec<u8>> {
    let img = image::RgbImage::from_fn(320, 240, |x, y| {
        if !(40..200).contains(&y) {
            image::Rgb([0, 0, 0])
        } else {
            let byte = |v: u32| u8::try_from((v + seed) % 256).unwrap_or(u8::MAX);
            image::Rgb([byte(x * 7), byte(y * 13), byte((x + y) * 3)])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    img.write_to(&mut bytes, image::ImageFormat::Jpeg)?;
    Ok(bytes.into_inner())
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(verbatim(dir)).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// A library at `dir\images`, with two folders: `Acme Inc.` (its name ends in a dot) and its sibling `Acme Inc`, each
/// with an `a.jpg`. Scanned, imported and with the borders found. Returns the catalog, the dotted folder's file and the
/// sibling's.
fn library(dir: &Path) -> Result<(Catalog, FileId, FileId)> {
    let images = dir.join("images");
    let (dotted, sibling) = (images.join("Acme Inc."), images.join("Acme Inc"));
    for (folder, seed) in [(&dotted, 0), (&sibling, 97)] {
        std::fs::create_dir_all(verbatim(folder))?;
        std::fs::write(verbatim(&folder.join("a.jpg")), jpeg(seed)?)?;
    }
    let mut c = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), PathRules::NATIVE)?;
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let root = pg_scan::normalize_path(&images)?;
    c.add_root(&root)?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    pg_import::import(&mut c, &opts, false, &ctl)?;
    pg_borders::find(&mut c, &opts, &ctl)?;
    let files = state::present_files(&c)?;
    anyhow::ensure!(files.len() == 2, "two files were expected: {files:?}");
    let (mut dotted_id, mut sibling_id) = (None, None);
    for (id, _) in &files {
        let path = c.file_path(*id)?.context("a file without a path")?;
        let in_dotted = path.parts.iter().rev().nth(1).is_some_and(|folder| folder.display().ends_with('.'));
        if in_dotted {
            dotted_id = Some(*id);
        } else {
            sibling_id = Some(*id);
        }
    }
    Ok((c, dotted_id.context("the file in the dotted folder")?, sibling_id.context("the sibling's file")?))
}

/// A source whose folder ends in a dot is refused: the sibling's copy is saved, the dotted file's isn't, and nothing is
/// left behind.
#[test]
fn a_source_in_a_folder_ending_in_a_dot_is_refused_and_leaves_no_temp() -> Result<()> {
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_crop", "folders-source")?)?;
    let (c, dotted, sibling) = library(&dir)?;
    let (dotted_original, sibling_original) = (
        std::fs::read(verbatim(&dir.join("images").join("Acme Inc.").join("a.jpg")))?,
        std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?,
    );
    let out = dir.join("out");
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = pg_crop::save(&c, &[dotted, sibling], &out, &opts, &Control::new())?;

    assert_eq!((stats.saved, stats.failed), (1, 1), "{:?}", stats.error);
    assert!(
        matches!(&stats.refused, Some(CropError::FolderRefused { folder, .. }) if folder == "Acme Inc."),
        "{:?}",
        stats.refused
    );
    assert!(stats.error.is_some(), "the refusal is also the first failure");
    assert_eq!(
        std::fs::read(verbatim(&dir.join("images").join("Acme Inc.").join("a.jpg")))?,
        dotted_original,
        "the refused file changed"
    );
    assert_eq!(
        std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?,
        sibling_original,
        "the sibling's file changed"
    );
    let copies = files_under(&out);
    assert_eq!(copies.len(), 1, "only the sibling's copy: {copies:?}");
    assert!(copies.iter().all(|p| !p.to_string_lossy().ends_with(".pg-crop")), "a temp is left: {copies:?}");
    drop(c);
    let _ = std::fs::remove_dir_all(verbatim(&dir));
    Ok(())
}

/// A copy into a folder whose name ends in a dot is refused before the folder is made: nothing is written, and the
/// source's file is untouched.
#[test]
fn a_copy_into_a_folder_ending_in_a_dot_is_refused_and_makes_nothing() -> Result<()> {
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_crop", "folders-copy")?)?;
    let (c, _dotted, sibling) = library(&dir)?;
    let original = std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?;
    let out = dir.join("Out.");
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = pg_crop::save(&c, &[sibling], &out, &opts, &Control::new())?;

    assert_eq!((stats.saved, stats.failed), (0, 1), "{:?}", stats.error);
    assert!(
        matches!(&stats.refused, Some(CropError::FolderRefused { folder, .. }) if folder == "Out."),
        "{:?}",
        stats.refused
    );
    assert!(!out.exists(), "the copies' folder was made");
    assert_eq!(std::fs::read(dir.join("images").join("Acme Inc").join("a.jpg"))?, original);
    drop(c);
    let _ = std::fs::remove_dir_all(verbatim(&dir));
    Ok(())
}
