//! Saving cropped copies of real files: borders found by the import, cut off into copies in the file's own format
//! with the original's modification time; originals untouched; marked files and changed files skipped; a folder
//! whose copies would land in the library refused. And cutting the borders off the originals themselves, in place,
//! once confirmed (`pg_crop::cut`): replaced, kept by NTFS identity, exact (a JPEG is re-encoded at its own quality, a
//! TIFF losslessly), its tags kept (ExifTool), its borders stored as none with no mark, refused without the confirmation,
//! a file another program holds left intact.
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, or the temp folder's `scratch_rate`), never in a dataset.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_codec::decode::{Decoder, MinSize};
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::path::PathArena;
use pg_model::query::{Query, Sort};
use pg_model::turns::Turns;

const W: u32 = 320;
const H: u32 = 240;
/// The letterbox: black bars this deep at the top and bottom.
const BAR: u32 = 40;

/// Photo-like content (blocks of hashed colours with fine noise, as the detector's own tests) between two black
/// bars.
fn letterboxed() -> image::RgbImage {
    let hash = |a: u32, b: u32| (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
    image::RgbImage::from_fn(W, H, |x, y| {
        if !(BAR..H - BAR).contains(&y) {
            return image::Rgb([0, 0, 0]);
        }
        let block = hash(x / 6, y / 6);
        let noise = hash(x, y) % 24;
        // 60 to 232, so it fits a channel.
        let c = |shift: u32| u8::try_from(60 + (block >> shift) % 150 + noise).unwrap_or(u8::MAX);
        image::Rgb([c(0), c(8), c(16)])
    })
}

fn make_files(dir: &Path) -> anyhow::Result<Vec<String>> {
    let img = letterboxed();
    let mut out = Vec::new();
    for (name, format) in [
        ("a.jpg", image::ImageFormat::Jpeg),
        ("b.png", image::ImageFormat::Png),
        ("c.webp", image::ImageFormat::WebP),
        ("d.tif", image::ImageFormat::Tiff),
        ("sub/e.bmp", image::ImageFormat::Bmp),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, format)?;
        let path = dir.join("images").join(name);
        std::fs::write(&path, bytes.get_ref())?;
        out.push(path.to_string_lossy().into_owned());
    }
    Ok(out)
}

fn ids(c: &Catalog) -> anyhow::Result<Vec<(i64, String)>> {
    Ok(state::present_files(c)?.into_iter().map(|(id, name)| (id.get(), name)).collect())
}

/// The place under `out` of the copies of the library folder `root`, as `pg_crop::save` puts them: the drive's letter as a
/// part (none on Unix, where the root is no part), then the folder's parts.
/// The copies' folder for a library folder: under `out`, the library folder's drive as a part, then its parts. The
/// library folder is taken as the catalog holds it (its canonical spelling), which is what the copies are written under.
fn place_in(out: &Path, root: &pg_model::rules::Path) -> PathBuf {
    let letter = String::from_utf8_lossy(root.anchor.as_bytes()).replace([':', '\\'], "");
    let mut place = out.to_path_buf();
    if !letter.is_empty() {
        place.push(letter);
    }
    for part in &root.parts {
        place.push(pg_platform::names::os_name(part));
    }
    place
}

#[test]
fn cropped_copies_cut_the_borders_and_leave_the_originals() {
    // The TIFF's metadata is copied with it.
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        pg_testkit::missing_tool("ExifTool");
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_rate", "crop", &["images/sub"]).unwrap();
    let files = make_files(&dir).unwrap();
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    for (id, _) in ids(&c).unwrap() {
        assert!(
            c.crop_source(FileId::new(id), &mut PathArena::default()).unwrap().is_none(),
            "the import doesn't look for borders"
        );
    }
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let files_ids = ids(&c).unwrap();
    assert_eq!(files_ids.len(), files.len());
    // The bars, and the safety margin past the seam.
    let mut cuts = std::collections::BTreeMap::new();
    for (id, name) in &files_ids {
        let source = c
            .crop_source(FileId::new(*id), &mut PathArena::default())
            .unwrap()
            .unwrap_or_else(|| panic!("{name}: no borders found"));
        let [l, t, r, b] = source.cut;
        assert!(
            l == 0 && r == 0 && (BAR..BAR + 3).contains(&t) && (BAR..BAR + 3).contains(&b),
            "{name}: {:?}",
            source.cut
        );
        cuts.insert(name.clone(), source.cut);
    }
    let originals: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let all: Vec<FileId> = files_ids.iter().map(|(id, _)| FileId::new(*id)).collect();

    // Into the library: refused, nothing written.
    assert!(
        pg_crop::save(&c, &all, &dir.join("images"), &opts, &ctl).is_err(),
        "copies into the library were not refused: folder {:?}, the library's root {:?}, the catalog's roots {:?}",
        dir.join("images"),
        root,
        c.roots().unwrap()
    );
    // Copies go into a folder named by their library folder's whole path, so saving into the parent doesn't reach the
    // library any more; a folder inside a library folder still does.
    assert!(pg_crop::save(&c, &all, &dir.join("images").join("sub"), &opts, &ctl).is_err(), "copies into the library");

    // One marked as not a border, one changed since: both skipped.
    let id_of = |name: &str| files_ids.iter().find(|(_, n)| n == name).unwrap().0;
    c.set_border_mark(FileId::new(id_of("b.png")), true).unwrap();
    let webp = &files[2];
    let mut changed = std::fs::read(webp).unwrap();
    changed.extend_from_slice(b"\0\0");
    std::fs::write(webp, &changed).unwrap();

    let out = dir.join("out");
    // The copies of the library folder `images`: under the copies' folder, in the library folder's place.
    let copies = place_in(&out, &root);
    let s = pg_crop::save(&c, &all, &out, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.skipped, s.failed), (3, 2, 0), "{:?}", s.failures);
    assert!(s.remaining.is_empty());
    for (i, path) in files.iter().enumerate() {
        if i != 2 {
            assert_eq!(&std::fs::read(path).unwrap(), &originals[i], "{path} changed");
        }
    }
    // The catalog keeps a file's name without its folder, so the cuts are keyed by the name alone.
    for name in ["a.jpg", "d.tif", "sub/e.bmp"] {
        let relative = Path::new(name);
        let key = relative.file_name().unwrap().to_string_lossy().into_owned();
        let copy = copies.join(relative);
        let bytes = std::fs::read(&copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
        let d = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, Turns::NONE).unwrap();
        // JPEG: the top cut moves inward to the 8 or 16 px grid, the bottom is exact.
        let [_, top, _, bottom] = cuts[&key];
        assert_eq!(u32::try_from(d.image.width).unwrap(), W, "{name}");
        let cut_top = H - bottom - u32::try_from(d.image.height).unwrap();
        if name == "a.jpg" {
            assert!(
                cut_top >= top && cut_top - top < 16 && cut_top.is_multiple_of(8),
                "{name}: top {top} cut at {cut_top}"
            );
        } else {
            assert_eq!(cut_top, top, "{name}");
        }
        let original = dir.join("images").join(name);
        assert_eq!(
            std::fs::metadata(&copy).unwrap().modified().unwrap(),
            std::fs::metadata(&original).unwrap().modified().unwrap(),
            "{name}: the copy keeps the original's time"
        );
        assert_eq!(Format::sniff(&bytes), Format::sniff(&std::fs::read(&original).unwrap()));
    }
    assert!(!copies.join("b.png").exists() && !copies.join("c.webp").exists());
}

/// The files of `make_files`' library folder, scanned, imported and looked at for borders: the catalog, and the files'
/// ids and names as the catalog has them.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn library_with_borders(dir: &Path) -> (Catalog, Vec<(i64, String)>) {
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let found = ids(&c).unwrap();
    (c, found)
}

/// Whether ExifTool can't be run here. The cuts of the JPEG, the TIFF and the WebP copy their tags with it, so their tests
/// are skipped without it, as the copies' test is.
fn exiftool_missing() -> bool {
    let missing = std::process::Command::new("exiftool").arg("-ver").output().is_err();
    if missing {
        pg_testkit::missing_tool("ExifTool");
    }
    missing
}

/// Write the ImageDescription `value` into `path`, with ExifTool.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn tag(path: &Path, value: &str) {
    let out = std::process::Command::new("exiftool")
        .arg("-overwrite_original")
        .arg(format!("-ImageDescription={value}"))
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// The ImageDescription ExifTool reads from `path`.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn description(path: &Path) -> String {
    let out = std::process::Command::new("exiftool").args(["-s3", "-ImageDescription"]).arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Cut off the originals once confirmed: each one is replaced by its cut (the same file, in its place), TIFF included,
/// and exact: the JPEG's top is cut where the borders are, not on a grid. The tags the JPEG and the TIFF carry survive
/// the cut. The catalog stores each cut file's borders as none, with no mark, so the next scan sees no change, no missing
/// file and no replacement (the file kept its identity); and the next import and border pass, which look at the cut
/// files, find no border.
#[test]
fn cut_originals_lose_their_borders_and_are_replaced_in_place() {
    if exiftool_missing() {
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_rate", "cut", &["images/sub"]).unwrap();
    let files = make_files(&dir).unwrap();
    tag(&dir.join("images").join("a.jpg"), "pg cut test");
    tag(&dir.join("images").join("d.tif"), "pg cut test");
    let (mut c, found) = library_with_borders(&dir);
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    let all: Vec<FileId> = found.iter().map(|(id, _)| FileId::new(*id)).collect();
    let originals: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let mut cuts = std::collections::BTreeMap::new();
    for (id, name) in &found {
        if let Some(source) = c.crop_source(FileId::new(*id), &mut PathArena::default()).unwrap() {
            cuts.insert(name.clone(), source.cut);
        }
    }

    let s = pg_crop::cut(&c, &all, true, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.skipped, s.unsupported, s.failed), (5, 0, 0, 0), "{:?}", s.failures);
    assert_eq!(s.cut.len(), 5);
    assert!(s.remaining.is_empty());

    for (i, path) in files.iter().enumerate() {
        let bytes = std::fs::read(path).unwrap();
        let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
        assert_ne!(bytes, originals[i], "{name}: not replaced");
        let d = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, Turns::NONE).unwrap();
        // The same frame as the copy keeps: the width is the stored one, and the cut heights add up. Exact: the top
        // is cut where the borders are, for the JPEG too.
        let [_, top, _, bottom] = cuts[&name];
        assert_eq!(u32::try_from(d.image.width).unwrap(), W, "{name}");
        let cut_top = H - bottom - u32::try_from(d.image.height).unwrap();
        assert_eq!(cut_top, top, "{name}: the cut is exact");
        assert_eq!(Format::sniff(&bytes), Format::sniff(&originals[i]), "{name}: the format stays");
    }
    assert_eq!(description(&dir.join("images").join("a.jpg")), "pg cut test", "the JPEG keeps its tag");
    assert_eq!(description(&dir.join("images").join("d.tif")), "pg cut test", "the TIFF keeps its tag");

    // Stored as none, with no mark: nothing to cut, no sides, and no review mark (the grid's borders are None).
    let all_entries = |c: &Catalog, q: Query| -> Vec<pg_model::grid::Entry> {
        c.list_grid(&pg_model::rules::Merged::default(), true, &q, Sort::default()).unwrap()
    };
    for (id, name) in &found {
        assert!(c.crop_source(FileId::new(*id), &mut PathArena::default()).unwrap().is_none(), "{name}: still to cut");
        assert!(c.border_sides(FileId::new(*id)).unwrap().is_empty(), "{name}: sides stored");
    }
    let shown = all_entries(&c, Query::default());
    assert_eq!(shown.len(), 5);
    assert!(shown.iter().all(|e| e.borders == pg_model::grid::Borders::None), "stored as none, not marked");
    assert!(all_entries(&c, Query { border_marked: Some(true), ..Query::default() }).is_empty(), "no mark");

    // The next scan sees them as they are: unchanged, not missing, not replaced.
    let s = pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!((s.changed, s.missing, s.pairing.replaced), (0, 0, 0));

    // The next import analyzes the cut files, and the border pass looks at them: it finds no border in any.
    pg_import::import(&mut c, &opts, false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_borders::find(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    for (id, name) in &found {
        assert!(c.border_sides(FileId::new(*id)).unwrap().is_empty(), "{name}: a border found in the cut");
        assert!(c.crop_source(FileId::new(*id), &mut PathArena::default()).unwrap().is_none(), "{name}: cut again");
    }
}

/// Refused without the user's confirmation: nothing is written, and the catalog still has the borders to cut.
#[test]
fn an_unconfirmed_cut_writes_nothing() {
    let dir = pg_testkit::run_folder_with("scratch_rate", "cut-unconfirmed", &["images/sub"]).unwrap();
    let files = make_files(&dir).unwrap();
    let (c, found) = library_with_borders(&dir);
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let all: Vec<FileId> = found.iter().map(|(id, _)| FileId::new(*id)).collect();
    let originals: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();

    assert!(pg_crop::cut(&c, &all, false, &opts, &ctl).is_err(), "a cut without the confirmation was not refused");
    for (path, original) in files.iter().zip(&originals) {
        assert_eq!(&std::fs::read(path).unwrap(), original, "{path} changed");
    }
    let a = found.iter().find(|(_, n)| n == "a.jpg").unwrap().0;
    assert!(c.crop_source(FileId::new(a), &mut PathArena::default()).unwrap().is_some());
    assert!(!dir.join("images").join(".a.jpg.cut.pg-write").exists());
}

/// A TIFF is cut in place like the others: re-encoded losslessly without its metadata, which ExifTool copies over before
/// the original is replaced, so its tag is still there. Its cut is exact, and it is still a TIFF.
#[test]
fn a_tiff_is_cut_and_keeps_its_metadata() {
    if exiftool_missing() {
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_rate", "cut-tiff", &["images/sub"]).unwrap();
    let files = make_files(&dir).unwrap();
    tag(Path::new(&files[3]), "pg tiff tag");
    let (c, found) = library_with_borders(&dir);
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let tiff = found.iter().find(|(_, n)| n == "d.tif").unwrap().0;
    let [_, top, _, bottom] = c.crop_source(FileId::new(tiff), &mut PathArena::default()).unwrap().unwrap().cut;
    let before = std::fs::read(&files[3]).unwrap();

    let s = pg_crop::cut(&c, &[FileId::new(tiff)], true, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.unsupported, s.failed), (1, 0, 0), "{:?}", s.failures);
    assert_eq!(s.cut, [FileId::new(tiff)]);
    let bytes = std::fs::read(&files[3]).unwrap();
    assert_ne!(bytes, before);
    assert_eq!(Format::sniff(&bytes), Some(Format::Tiff));
    let d = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, Turns::NONE).unwrap();
    assert_eq!(
        (u32::try_from(d.image.width).unwrap(), H - bottom - u32::try_from(d.image.height).unwrap()),
        (W, top),
        "exact"
    );
    assert_eq!(description(Path::new(&files[3])), "pg tiff tag", "the metadata is copied over");
    assert!(
        std::fs::read_dir(dir.join("images"))
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains("cut")),
        "no temporary is left beside the original"
    );
}

/// A failure before the copy into the original leaves the original as it was, and the temp is gone. Another program holds
/// `a.jpg` open, sharing it with readers only, so the cut can't open it for writing: that file fails, and the others are
/// cut.
#[cfg(windows)]
#[test]
fn a_failure_mid_write_leaves_the_original_intact_and_the_rest_go_on() {
    use std::os::windows::fs::OpenOptionsExt;
    if exiftool_missing() {
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_rate", "cut-held", &["images/sub"]).unwrap();
    let files = make_files(&dir).unwrap();
    let (c, found) = library_with_borders(&dir);
    let ctl = Control::new();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let all: Vec<FileId> = found.iter().map(|(id, _)| FileId::new(*id)).collect();
    let originals: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();

    // FILE_SHARE_READ only: no other handle may write the file while this one is open.
    let holder = std::fs::OpenOptions::new().read(true).share_mode(0x1).open(&files[0]).unwrap();
    let s = pg_crop::cut(&c, &all, true, &opts, &ctl).unwrap();
    drop(holder);

    assert_eq!((s.saved, s.unsupported, s.failed), (4, 0, 1), "{:?}", s.failures);
    assert!(s.failures[0].path.ends_with("a.jpg"), "{:?}", s.failures);
    assert_eq!(std::fs::read(&files[0]).unwrap(), originals[0], "the held original is intact");
    assert!(!dir.join("images").join(".a.jpg.cut.pg-write").exists(), "the temp is removed");
    assert_ne!(std::fs::read(&files[1]).unwrap(), originals[1], "the rest are cut");
}
