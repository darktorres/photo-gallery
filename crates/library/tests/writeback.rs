//! Write-back on real files: ratings into each format, lossless turns, file identity kept, no re-import after a
//! metadata write, and a rating changed by another program adopted. Needs ExifTool (skipped without it).
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, default `D:\PhotoGallery\scratch_rate`, never C:), never
//! in a dataset.

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_codec::decode::{Decoder, MinSize, orient};
use pg_codec::pixels::Rgb;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_library::writeback::Writer;
use pg_library::{import, scan};
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::turns::Turns;
use pg_testkit::events::Quiet;
use pg_thumbcache::ThumbCache;

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
    pg_render::display::decode(&mut Decoder::new().unwrap(), &std::fs::read(path).unwrap(), MinSize::Full, Turns::NONE)
        .unwrap()
        .image
}

fn ids(c: &Catalog) -> Vec<(i64, String)> {
    state::present_files(c).into_iter().map(|(id, name)| (id.get(), name)).collect()
}

fn wait_for_queue(c: &Catalog) {
    let start = Instant::now();
    loop {
        if state::writes_still_tried(c) == 0 {
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
    // Earlier runs' folders go first, and this run's thumbnail cache stays open to the end, so it can't remove its own.
    let dir = pg_testkit::run_folder_with(r"D:\PhotoGallery\scratch_rate", "writeback", &["images"]);
    let files = make_files(&dir);
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions()).unwrap();
    let ctl = Control::new();
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    import::import(&mut c, &opts, false, &ctl).unwrap();
    let before = ids(&c);
    assert_eq!(before.len(), files.len());

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs);

    // Ratings: every format but BMP gets it in the file; nothing needs re-importing afterwards.
    for (n, (id, _)) in before.iter().enumerate() {
        pg_catalog::writes::set_rating(&c, FileId::new(*id), Rating::new((n % 5 + 1) as u8), true).unwrap();
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
        pg_catalog::writes::queue_rotation(&c, FileId::new(*id), Turns::new(1)).unwrap();
    }
    writer.wake();
    wait_for_queue(&c);
    assert_eq!(writer.status().error, None);
    for ((path, format), original) in files.iter().zip(originals) {
        let turned = orient(original, pg_model::orientation::Orientation::new(6).unwrap());
        let rotation = state::file_rotation(&c, &Path::new(path).file_name().unwrap().to_string_lossy());
        let bytes = std::fs::read(path).unwrap();
        let now = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, Turns::new(rotation))
            .unwrap()
            .image;
        assert_eq!((now.width, now.height), (turned.width, turned.height), "{path}");
        if matches!(format, Format::Png | Format::Gif | Format::Bmp | Format::Tiff) {
            assert_eq!(now.pixels, turned.pixels, "{path}: lossless");
        }
        // Ratings survive turning.
        if *format != Format::Bmp {
            assert!(Meta::read(&bytes, *format, &mut MetaScratch::default()).rating.is_some(), "{path}: rating kept");
        }
    }
    let dims = |c: &Catalog| state::analysis_sizes(c);
    // The grid lays them out turned at once, before the analysis is redone, and shows their placeholders turned:
    // the turn is in the file, or kept in the catalog.
    assert!(dims(&c).iter().all(|&d| d == (48, 64)), "sizes right after the turn: {:?}", dims(&c));
    let placeholder_turns = |c: &Catalog| -> Vec<u8> {
        before
            .iter()
            .map(|(id, _)| pg_catalog::browse::read::placeholder(c, FileId::new(*id)).unwrap().unwrap().turns.get())
            .collect()
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
    assert_eq!(state::rating_of(&c, FileId::new(before[0].0)), Some(2));

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A finishing copy whose original is locked (another program has it open without sharing) fails at every try. The
/// queue waits longer between the tries, gives up at MAX_ATTEMPTS, and discards the temp: the file then isn't shown as
/// being written, and the failure stays in the queue. Without the give-up, the job was retried at once, forever.
#[cfg(windows)]
#[test]
fn a_finishing_copy_that_keeps_failing_is_given_up() {
    use std::os::windows::fs::OpenOptionsExt;

    // Its own folder: run_folder keeps one folder per prefix, and this test runs beside the others in the binary.
    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_rate", "giveup");
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir);
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions()).unwrap();
    let ctl = Control::new();
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // The file as the database has it, and the temp a write would have left beside it (writeback's temp_path).
    let (id, db_path) = state::file_and_path(&c, "b.png");
    let (folder, name) = db_path.rsplit_once('\\').unwrap();
    let temp = format!("{folder}\\.{name}.pg-write");
    std::fs::copy(&files[1].0, &temp).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp);

    // Locked: the copy into the original can't open it.
    let _lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&files[1].0).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs);
    writer.wake();

    // Given up: the temp is gone from the queue and from disk.
    let start = Instant::now();
    loop {
        let row = state::write_row(&c, id);
        if row.attempts >= 5 && row.temp.is_none() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "not given up at {} attempts", row.attempts);
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!Path::new(&temp).exists(), "the temp is still on disk");
    assert!(writer.status().error.is_some());
    let error = state::write_row(&c, id).error;
    assert!(error.is_some(), "the failure is gone from the queue");
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);

    // And it stays given up: no more tries.
    std::thread::sleep(Duration::from_secs(3));
    let attempts = state::write_row(&c, id).attempts;
    assert_eq!(attempts, 5);

    drop(_lock);
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

#[test]
fn a_write_for_a_file_that_is_gone_is_dropped_and_a_deleted_one_is_counted() {
    // Its own folder, as the other tests here: run_folder keeps one folder per prefix.
    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_rate", "gone");
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir);
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions()).unwrap();
    let ctl = Control::new();
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();
    // a.jpg: a scan has marked it missing, so its write is dropped. b.png was deleted outside the app while the
    // catalog still lists it: its write is tried, and each failure is counted (it isn't waited on as offline).
    let (marked, deleted) = (state::file_id(&c, "a.jpg"), state::file_id(&c, "b.png"));
    state::mark_missing(&c, marked);
    std::fs::remove_file(&files[1].0).unwrap();
    for file in [marked, deleted] {
        state::queue_write(&c, file, "rating", 3);
    }

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs);
    writer.wake();

    let start = Instant::now();
    loop {
        let queued = state::writes_queued(&c, marked);
        let attempts = state::write_row(&c, deleted).attempts;
        if queued == 0 && attempts >= 2 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "queued {queued}, deleted file's attempts {attempts}");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(writer.status().error.is_some(), "the cause is in the status line");
    quiet.check();
}

#[test]
fn a_finished_rotation_copy_turns_the_analysis_back_as_a_normal_write_does() {
    // Its own folder, as the other tests here: run_folder keeps one folder per prefix.
    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_rate", "finish");
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir);
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions()).unwrap();
    let ctl = Control::new();
    let root = scan::normalize_path(&dir.join("images").to_string_lossy()).unwrap();
    scan::scan(&mut c, &root, &ctl).unwrap();
    import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // A turn of b.png whose copy was complete when the app stopped: its temp is beside it, and the analysis still
    // shows the turn pending (rotation 1).
    let (id, db_path) = state::file_and_path(&c, "b.png");
    let (folder, name) = db_path.rsplit_once('\\').unwrap();
    let temp = format!("{folder}\\.{name}.pg-write");
    std::fs::copy(&files[1].0, &temp).unwrap();
    state::set_analysis_rotation(&c, id, 1);
    state::queue_finished_copy(&c, id, "rotation", 1, &temp);

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs);
    writer.wake();

    let start = Instant::now();
    loop {
        if state::writes_queued(&c, id) == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the finished copy is still queued");
        std::thread::sleep(Duration::from_millis(200));
    }
    let rotation = state::analysis_rotation(&c, id);
    assert_eq!(rotation, 0, "the turn is in the file, so the analysis no longer shows it pending");
    quiet.check();
}
