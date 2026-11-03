//! Saving cropped copies of real files: borders found by the import, cut off into copies in the file's own format
//! with the original's modification time; originals untouched; marked files and changed files skipped; a folder
//! whose copies would land in the library refused.
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, default `D:\PhotoGallery\scratch_rate`, never C:), never
//! in a dataset.

use std::io::Cursor;
use std::path::Path;

use pg_catalog::Catalog;
use pg_codec::decode::{Decoder, MinSize};
use pg_ingest::control::Control;
use pg_ingest::pipeline::Options;
use pg_ingest::{borders, crop, import, scan};
use pg_model::format::Format;
use pg_model::path::PathArena;

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
        let noise = (hash(x, y) % 24) as i32;
        let c = |shift: u32| (60 + (block >> shift) % 150) as i32 + noise;
        image::Rgb([c(0) as u8, c(8) as u8, c(16) as u8])
    })
}

fn make_files(dir: &Path) -> Vec<String> {
    let img = letterboxed();
    let mut out = Vec::new();
    for (name, format) in [
        ("a.jpg", image::ImageFormat::Jpeg),
        ("b.png", image::ImageFormat::Png),
        ("c.webp", image::ImageFormat::WebP),
        ("d.tif", image::ImageFormat::Tiff),
        (r"sub\e.bmp", image::ImageFormat::Bmp),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, format).unwrap();
        let path = dir.join("images").join(name);
        std::fs::write(&path, bytes.get_ref()).unwrap();
        out.push(path.to_string_lossy().into_owned());
    }
    out
}

fn ids(c: &Catalog) -> Vec<(i64, String)> {
    let mut stmt = c.conn.prepare("SELECT id, name FROM files WHERE missing = 0 ORDER BY name").unwrap();
    stmt.query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap()))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn cropped_copies_cut_the_borders_and_leave_the_originals() {
    // The TIFF's metadata is copied with it.
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        eprintln!("ExifTool not found: skipped");
        return;
    }
    let dir = pg_testkit::run_folder_with(r"D:\PhotoGallery\scratch_rate", "crop", &["images/sub"]);
    let files = make_files(&dir);
    let mut c = Catalog::open(&dir.join("catalog.sqlite")).unwrap();
    let ctl = Control::new(false);
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    c.add_root(&root).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    import::import(&mut c, &opts, false, &ctl).unwrap();
    for (id, _) in ids(&c) {
        assert!(c.crop_source(id, &mut PathArena::default()).unwrap().is_none(), "the import doesn't look for borders");
    }
    borders::find(&mut c, &opts, &ctl).unwrap();
    let files_ids = ids(&c);
    assert_eq!(files_ids.len(), files.len());
    // The bars, and the safety margin past the seam.
    let mut cuts = std::collections::BTreeMap::new();
    for (id, name) in &files_ids {
        let source = c
            .crop_source(*id, &mut PathArena::default())
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
    let all: Vec<i64> = files_ids.iter().map(|(id, _)| *id).collect();

    // Into the library: refused, nothing written.
    assert!(crop::save(&c, &all, &dir.join("images"), &opts, &ctl).is_err());
    // Copies go into a folder named by their library folder's whole path, so saving into the parent doesn't reach the
    // library any more; a folder inside a library folder still does.
    assert!(crop::save(&c, &all, &dir.join("images").join("sub"), &opts, &ctl).is_err(), "copies into the library");

    // One marked as not a border, one changed since: both skipped.
    let id_of = |name: &str| files_ids.iter().find(|(_, n)| n == name).unwrap().0;
    pg_catalog::Catalog::set_border_mark(&c.conn, id_of("b.png"), true).unwrap();
    let webp = &files[2];
    let mut changed = std::fs::read(webp).unwrap();
    changed.extend_from_slice(b"\0\0");
    std::fs::write(webp, &changed).unwrap();

    let out = dir.join("out");
    // The copies of the library folder `images`: under the copies' folder, in the library folder's place (its path,
    // without the colon).
    let copies = out.join(dir.join("images").to_string_lossy().replace(':', ""));
    let s = crop::save(&c, &all, &out, &opts, &ctl).unwrap();
    assert_eq!((s.saved, s.skipped, s.failed), (3, 2, 0), "{:?}", s.error);
    assert!(s.remaining.is_empty());
    for (i, path) in files.iter().enumerate() {
        if i != 2 {
            assert_eq!(&std::fs::read(path).unwrap(), &originals[i], "{path} changed");
        }
    }
    for name in ["a.jpg", "d.tif", r"sub\e.bmp"] {
        let copy = copies.join(name);
        let bytes = std::fs::read(&copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
        let d = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, 0).unwrap();
        // JPEG: the top cut moves inward to the 8 or 16 px grid, the bottom is exact.
        let [_, top, _, bottom] = cuts[name.rsplit('\\').next().unwrap()];
        assert_eq!(d.image.width as u32, W, "{name}");
        let cut_top = H - bottom - d.image.height as u32;
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
