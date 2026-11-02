//! Write-back on real files: ratings into each format, lossless turns, file identity kept, no re-import after a
//! metadata write, and a rating changed by another program adopted. Needs ExifTool (skipped without it).
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, default `D:\PhotoGallery\scratch_rate`, never C:), never
//! in a dataset.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pg_core::catalog::Catalog;
use pg_core::decode::{Decoder, MinSize, Rgb, orient};
use pg_core::format::Format;
use pg_core::metadata::{Meta, MetaScratch};
use pg_core::thumbs::ThumbCache;
use pg_ingest::control::Control;
use pg_ingest::pipeline::Options;
use pg_ingest::writeback::Writer;
use pg_ingest::{import, scan};

fn scratch() -> PathBuf {
    // Earlier runs' folders go first, and this run's thumbnail cache stays open to the end, so it can't remove its own.
    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_rate", "writeback");
    std::fs::create_dir_all(dir.join("images")).unwrap();
    dir
}

fn pattern(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, ((x * 7 + y * 13) % 256) as u8])
    })
}

/// One file per format, sized so the JPEG transform is perfect (multiples of 16).
fn make_files(dir: &Path) -> Vec<(String, Format)> {
    let img = pattern(64, 48);
    let mut out = Vec::new();
    for (name, format) in [
        ("a.jpg", image::ImageFormat::Jpeg),
        ("b.png", image::ImageFormat::Png),
        ("c.webp", image::ImageFormat::WebP),
        ("d.tif", image::ImageFormat::Tiff),
        ("e.gif", image::ImageFormat::Gif),
        ("f.bmp", image::ImageFormat::Bmp),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, format).unwrap();
        let path = dir.join("images").join(name);
        std::fs::write(&path, bytes.get_ref()).unwrap();
        out.push((path.to_string_lossy().into_owned(), Format::sniff(bytes.get_ref()).unwrap()));
    }
    out
}

fn shown(path: &str) -> Rgb {
    pg_core::display::decode(&mut Decoder::new().unwrap(), &std::fs::read(path).unwrap(), MinSize::Full, 0)
        .unwrap()
        .image
}

fn ids(c: &Catalog) -> Vec<(i64, String)> {
    let mut stmt = c.conn.prepare("SELECT id, name FROM files WHERE missing = 0 ORDER BY name").unwrap();
    stmt.query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap()))).unwrap().map(Result::unwrap).collect()
}

fn wait_for_queue(c: &Catalog) {
    let start = Instant::now();
    loop {
        let left: i64 =
            c.conn.query_row("SELECT count(*) FROM file_writes WHERE attempts < 5", [], |r| r.get(0)).unwrap();
        if left == 0 {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "writes still queued");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn ratings_and_turns_reach_the_files() {
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        eprintln!("ExifTool not found: skipped");
        return;
    }
    let dir = scratch();
    let files = make_files(&dir);
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::open(&catalog_path).unwrap();
    let ctl = Control::new(false);
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(0);
    import::import(&mut c, &opts, false, &ctl).unwrap();
    let before = ids(&c);
    assert_eq!(before.len(), files.len());

    let thumbs: &'static ThumbCache = Box::leak(Box::new(
        ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), Box::new(|e| panic!("{e:#}"))).unwrap(),
    ));
    let writer = Writer::start(catalog_path.clone(), thumbs, || {});

    // Ratings: every format but BMP gets it in the file; nothing needs re-importing afterwards.
    for (n, (id, _)) in before.iter().enumerate() {
        pg_core::browse::writes::set_rating(&c.conn, *id, (n % 5 + 1) as u8, true).unwrap();
    }
    writer.wake();
    wait_for_queue(&c);
    assert_eq!(writer.status().error, None);
    for (n, (path, format)) in files.iter().enumerate() {
        let expected = if *format == Format::Bmp { None } else { Some((n % 5 + 1) as u8) };
        assert_eq!(
            Meta::read(&std::fs::read(path).unwrap(), *format, &mut MetaScratch::default()).rating,
            expected,
            "{path}"
        );
    }
    assert_eq!(c.status().unwrap().pending, 0, "a metadata write must not trigger a re-import");
    // Same files, same identities (written in place).
    scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c), before);
    assert_eq!(c.status().unwrap().missing, 0);

    // A turn: the files show turned; the analysis is redone with the new size.
    let originals: Vec<Rgb> = files.iter().map(|(p, _)| shown(p)).collect();
    for (id, _) in &before {
        pg_core::browse::writes::queue_rotation(&c.conn, *id, 1).unwrap();
    }
    writer.wake();
    wait_for_queue(&c);
    assert_eq!(writer.status().error, None);
    for ((path, format), original) in files.iter().zip(originals) {
        let turned = orient(original, 6);
        let rotation: u8 = c
            .conn
            .query_row(
                "SELECT rotation FROM files WHERE name = ?1",
                [Path::new(path).file_name().unwrap().to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap();
        let bytes = std::fs::read(path).unwrap();
        let now =
            pg_core::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, rotation).unwrap().image;
        assert_eq!((now.width, now.height), (turned.width, turned.height), "{path}");
        if matches!(format, Format::Png | Format::Gif | Format::Bmp | Format::Tiff) {
            assert_eq!(now.pixels, turned.pixels, "{path}: lossless");
        }
        // Ratings survive turning.
        if *format != Format::Bmp {
            assert!(Meta::read(&bytes, *format, &mut MetaScratch::default()).rating.is_some(), "{path}: rating kept");
        }
    }
    let dims = |c: &Catalog| -> Vec<(u32, u32)> {
        c.conn
            .prepare("SELECT a.width, a.height FROM analysis a JOIN files f ON f.id = a.file_id ORDER BY f.name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    // The grid lays them out turned at once, before the analysis is redone, and shows their placeholders turned:
    // the turn is in the file, or kept in the catalog.
    assert!(dims(&c).iter().all(|&d| d == (48, 64)), "sizes right after the turn: {:?}", dims(&c));
    let placeholder_turns = |c: &Catalog| -> Vec<u8> {
        before.iter().map(|(id, _)| pg_core::browse::read::placeholder(&c.conn, *id).unwrap().unwrap().turns).collect()
    };
    assert!(placeholder_turns(&c).iter().all(|&t| t == 1), "placeholders after the turn: {:?}", placeholder_turns(&c));
    scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c), before);
    import::import(&mut c, &opts, false, &ctl).unwrap();
    assert!(dims(&c).iter().all(|&d| d == (48, 64)), "re-imported upright sizes: {:?}", dims(&c));
    assert!(placeholder_turns(&c).iter().all(|&t| t == 0), "re-imported placeholders: {:?}", placeholder_turns(&c));

    // Another program rates the JPEG: the next scan and import adopt it.
    let jpg = &files[0].0;
    let out = std::process::Command::new("exiftool")
        .args(["-overwrite_original_in_place", "-XMP:Rating=2", jpg])
        .output()
        .unwrap();
    assert!(out.status.success());
    scan::scan(&mut c, &root, &ctl).unwrap();
    import::import(&mut c, &opts, false, &ctl).unwrap();
    let rating: u8 =
        c.conn.query_row("SELECT rating FROM ratings WHERE file_id = ?1", [before[0].0], |r| r.get(0)).unwrap();
    assert_eq!(rating, 2);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
}
