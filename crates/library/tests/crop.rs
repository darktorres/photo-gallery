//! Saving cropped copies of real files: borders found by the import, cut off into copies in the file's own format
//! with the original's modification time; originals untouched; marked files and changed files skipped; a folder
//! whose copies would land in the library refused.
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
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    for (id, _) in ids(&c).unwrap() {
        assert!(
            c.crop_source(FileId::new(id), &mut PathArena::default()).unwrap().is_none(),
            "the import doesn't look for borders"
        );
    }
    pg_borders::find(&mut c, &opts, &ctl).unwrap();
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
    assert_eq!((s.saved, s.skipped, s.failed), (3, 2, 0), "{:?}", s.error);
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
