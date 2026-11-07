//! Write-back on real files: ratings into each format, lossless turns, file identity kept, no re-import after a
//! metadata write, and a rating changed by another program adopted. Needs ExifTool (skipped without it).
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, default `D:\PhotoGallery\scratch_rate`, never C:), never
//! in a dataset.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_codec::decode::{Decoder, MinSize, orient};
use pg_codec::pixels::Rgb;
use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_jobs::pipeline::Options;
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;
use pg_testkit::events::Quiet;
use pg_thumbcache::ThumbCache;
use pg_writeback::Writer;

fn pattern(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        // Each channel is below 256: x < w, y < h.
        let byte = |v: u32| u8::try_from(v).unwrap_or(u8::MAX);
        image::Rgb([byte(x * 255 / w), byte(y * 255 / h), byte((x * 7 + y * 13) % 256)])
    })
}

/// One file per format, sized so the JPEG transform is perfect (multiples of 16).
fn make_files(dir: &Path) -> anyhow::Result<Vec<(String, Format)>> {
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
        img.write_to(&mut bytes, format)?;
        let path = dir.join("images").join(name);
        std::fs::write(&path, bytes.get_ref())?;
        let sniffed = Format::sniff(bytes.get_ref()).ok_or_else(|| anyhow!("{name}: not a format the codec knows"))?;
        out.push((path.to_string_lossy().into_owned(), sniffed));
    }
    Ok(out)
}

fn shown(path: &str) -> anyhow::Result<Rgb> {
    Ok(pg_render::display::decode(&mut Decoder::new()?, &std::fs::read(path)?, MinSize::Full, Turns::NONE)?.image)
}

fn ids(c: &Catalog) -> anyhow::Result<Vec<(i64, String)>> {
    Ok(state::present_files(c)?.into_iter().map(|(id, name)| (id.get(), name)).collect())
}

fn wait_for_queue(c: &Catalog) -> anyhow::Result<()> {
    let start = Instant::now();
    loop {
        if state::writes_still_tried(c)? == 0 {
            return Ok(());
        }
        assert!(start.elapsed() < Duration::from_secs(60), "writes still queued");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[cfg_attr(not(windows), ignore = "S-17: the catalog joins folder and name with a Windows separator")]
fn ratings_and_turns_reach_the_files() {
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        pg_testkit::missing_tool("ExifTool");
        return;
    }
    // Earlier runs' folders go first, and this run's thumbnail cache stays open to the end, so it can't remove its own.
    let dir = pg_testkit::run_folder_with(r"D:\PhotoGallery\scratch_rate", "writeback", &["images"]).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    let before = ids(&c).unwrap();
    assert_eq!(before.len(), files.len());

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();

    // Ratings: every format but BMP gets it in the file; nothing needs re-importing afterwards.
    for (n, (id, _)) in before.iter().enumerate() {
        let rating = Rating::new(u8::try_from(n % 5 + 1).unwrap());
        pg_catalog::writes::set_rating(&c, FileId::new(*id), rating, true).unwrap();
    }
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    for (n, (path, format)) in files.iter().enumerate() {
        let expected = if *format == Format::Bmp { None } else { Some(u8::try_from(n % 5 + 1).unwrap()) };
        assert_eq!(
            Meta::read(&std::fs::read(path).unwrap(), *format, &mut MetaScratch::default()).rating,
            expected,
            "{path}"
        );
    }
    assert_eq!(c.status().unwrap().pending, 0, "a metadata write must not trigger a re-import");
    // Same files, same identities (written in place).
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    assert_eq!(c.status().unwrap().missing, 0);

    // A turn: the files show turned; the analysis is redone with the new size.
    let originals: Vec<Rgb> = files.iter().map(|(p, _)| shown(p).unwrap()).collect();
    for (id, _) in &before {
        pg_catalog::writes::queue_rotation(&c, FileId::new(*id), Turns::new(1)).unwrap();
    }
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    for ((path, format), original) in files.iter().zip(originals) {
        let turned = orient(original, pg_model::orientation::Orientation::new(6).unwrap());
        let rotation = state::file_rotation(&c, &Path::new(path).file_name().unwrap().to_string_lossy()).unwrap();
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
    let dims = |c: &Catalog| state::analysis_sizes(c).unwrap();
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
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    assert!(dims(&c).iter().all(|&d| d == (48, 64)), "re-imported upright sizes: {:?}", dims(&c));
    assert!(placeholder_turns(&c).iter().all(|&t| t == 0), "re-imported placeholders: {:?}", placeholder_turns(&c));

    // Another program rates the JPEG: the next scan and import adopt it.
    let jpg = &files[0].0;
    let out = std::process::Command::new("exiftool")
        .args(["-overwrite_original_in_place", "-XMP:Rating=2", jpg])
        .output()
        .unwrap();
    assert!(out.status.success());
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    assert_eq!(state::rating_of(&c, FileId::new(before[0].0)).unwrap(), Some(2));

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A JPEG and a video whose names have no UTF-8 form (a lone surrogate on Windows, a raw byte on Unix) are imported, and
/// the rating reaches the JPEG in place: the write goes by the exact name, the files keep their identities, and the
/// folder holds no other file. Needs ExifTool; skipped without it.
#[test]
fn a_file_whose_name_is_not_utf8_is_imported_and_rated_in_place() {
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        pg_testkit::missing_tool("ExifTool");
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_writeback", "odd-names", &["images"]).unwrap();
    let (jpeg_name, video_name): (&[u8], &[u8]) = if cfg!(windows) {
        (b"odd\xED\xA0\x80.jpg", b"clip\xED\xA0\x80.mp4")
    } else {
        (b"odd\xFF.jpg", b"clip\xFF.mp4")
    };
    let images = dir.join("images");
    let jpeg = images.join(pg_platform::names::path_buf(jpeg_name));
    let mut bytes = Cursor::new(Vec::new());
    pattern(64, 48).write_to(&mut bytes, image::ImageFormat::Jpeg).unwrap();
    std::fs::write(&jpeg, bytes.get_ref()).unwrap();
    // The clip from the video crate's tests: 64x48, six frames.
    std::fs::write(
        images.join(pg_platform::names::path_buf(video_name)),
        include_bytes!("../../video/tests/sample.mp4"),
    )
    .unwrap();

    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&images).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    let before = ids(&c).unwrap();
    assert_eq!(before.len(), 2, "both imported: {before:?}");
    let jpeg_id = before.iter().find(|(_, name)| name.ends_with(".jpg")).expect("the JPEG").0;

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    // The thumbnail pass makes both: the JPEG's, and the clip's (read by the video reader from its own bytes).
    let made = pg_thumbs::run(thumbs, &catalog_path, 64, &opts, &ctl).unwrap();
    assert_eq!((made.made, made.failed), (2, 0), "thumbnails of both files");
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    pg_catalog::writes::set_rating(&c, FileId::new(jpeg_id), Rating::new(4), true).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    assert_eq!(Meta::read(&std::fs::read(&jpeg).unwrap(), Format::Jpeg, &mut MetaScratch::default()).rating, Some(4));
    // Written in place: the same identities, and no other file beside them.
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    assert_eq!(std::fs::read_dir(&images).unwrap().count(), 2);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(&images);
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
    let dir = pg_testkit::run_folder("scratch_rate", "giveup").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // The file as the database has it, and the temp a write would have left beside it (writeback's temp_of).
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path);
    std::fs::copy(&files[1].0, pg_platform::names::path_buf(&c.rules().join(&temp))).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();

    // Locked: the copy into the original can't open it.
    let _lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&files[1].0).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();

    // Given up: the temp is gone from the queue and from disk.
    let start = Instant::now();
    loop {
        let row = state::write_row(&c, id).unwrap();
        if row.attempts >= 5 && row.temp.is_none() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "not given up at {} attempts", row.attempts);
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!pg_platform::names::path_buf(&c.rules().join(&temp)).exists(), "the temp is still on disk");
    assert!(writer.status().error.is_some());
    let error = state::write_row(&c, id).unwrap().error;
    assert!(error.is_some(), "the failure is gone from the queue");
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);

    // And it stays given up: no more tries.
    std::thread::sleep(Duration::from_secs(3));
    let attempts = state::write_row(&c, id).unwrap().attempts;
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
    let dir = pg_testkit::run_folder("scratch_rate", "gone").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();
    // a.jpg: a scan has marked it missing, so its write is dropped. b.png was deleted outside the app while the
    // catalog still lists it: its write is tried, and each failure is counted (it isn't waited on as offline).
    let (marked, deleted) = (state::file_id(&c, "a.jpg").unwrap(), state::file_id(&c, "b.png").unwrap());
    state::mark_missing(&c, marked).unwrap();
    std::fs::remove_file(&files[1].0).unwrap();
    for file in [marked, deleted] {
        state::queue_write(&c, file, "rating", 3).unwrap();
    }

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();

    let start = Instant::now();
    loop {
        let queued = state::writes_queued(&c, marked).unwrap();
        let attempts = state::write_row(&c, deleted).unwrap().attempts;
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
#[cfg_attr(not(windows), ignore = "S-17: the catalog joins folder and name with a Windows separator")]
fn a_finished_rotation_copy_turns_the_analysis_back_as_a_normal_write_does() {
    // Its own folder, as the other tests here: run_folder keeps one folder per prefix.
    let dir = pg_testkit::run_folder("scratch_rate", "finish").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // A turn of b.png whose copy was complete when the app stopped: its temp is beside it, and the analysis still
    // shows the turn pending (rotation 1).
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path);
    std::fs::copy(&files[1].0, pg_platform::names::path_buf(&c.rules().join(&temp))).unwrap();
    state::set_analysis_rotation(&c, id, 1).unwrap();
    state::queue_finished_copy(&c, id, "rotation", 1, &temp).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();

    let start = Instant::now();
    loop {
        if state::writes_queued(&c, id).unwrap() == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the finished copy is still queued");
        std::thread::sleep(Duration::from_millis(200));
    }
    let rotation = state::analysis_rotation(&c, id).unwrap();
    assert_eq!(rotation, 0, "the turn is in the file, so the analysis no longer shows it pending");
    quiet.check();
}

/// A finishing copy queued while its temp was text (the catalog before temps were kept as bytes) is carried over by the
/// upgrade and finished: the original takes the temp's content, the rating is applied, and the temp is gone.
#[test]
fn a_finishing_copy_carried_from_a_text_temp_is_finished() {
    let dir = pg_testkit::run_folder_with("scratch_writeback", "carried-temp", &["images"]).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // The temp a rating of b.png left: it holds a.jpg's bytes, so the copy into b.png is visible.
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path);
    let temp_text = String::from_utf8(c.rules().join(&temp)).unwrap();
    let copied = std::fs::read(&files[0].0).unwrap();
    std::fs::write(pg_platform::names::path_buf(&c.rules().join(&temp)), &copied).unwrap();
    drop(c);

    // The catalog as it was before, with the temp as text.
    let old = rusqlite::Connection::open(&catalog_path).unwrap();
    old.execute_batch(
        r"DROP TABLE file_writes;
          CREATE TABLE file_writes(
              file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
              kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
              value    INTEGER NOT NULL,
              queued   INTEGER NOT NULL,
              attempts INTEGER NOT NULL DEFAULT 0,
              error    TEXT,
              temp       TEXT,
              temp_value INTEGER,
              PRIMARY KEY(file_id, kind));",
    )
    .unwrap();
    old.execute(
        "INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value) VALUES (?1, 'rating', 3, 0, ?2, 3)",
        rusqlite::params![id.get(), temp_text],
    )
    .unwrap();
    drop(old);

    // The upgrade carries the row (its temp as bytes), and the writer finishes the copy.
    let c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    assert!(c.rebuilt().is_some(), "the text layout is moved aside");
    assert_eq!(state::write_row(&c, id).unwrap().temp, Some(temp.clone()));
    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    assert_eq!(std::fs::read(&files[1].0).unwrap(), copied, "the copy went into the original");
    assert!(!pg_platform::names::path_buf(&c.rules().join(&temp)).exists(), "the temp is still on disk");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the file holds the rating once the copy is done");

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}
