//! Write-back on real files: ratings into each format, lossless turns, file identity kept, no re-import after a
//! metadata write, and a rating changed by another program adopted. Needs ExifTool (skipped without it).
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, or the temp folder's `scratch_rate`), never in a dataset.

use std::io::Cursor;
use std::path::{Path, PathBuf};
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
use pg_jobs_testkit::Quiet;
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::turns::Turns;
use pg_model::write::WriteKind;
use pg_testkit::held::{Access, hold};
use pg_thumbcache::ThumbCache;
use pg_writeback::fault::Fault;
use pg_writeback::{WriteError, Writer};

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

/// A library of [`make_files`] in a folder of its own (`prefix`; each run_folder call gets a folder of its own, and these
/// tests run beside each other), scanned and imported: the folder, the files, the catalog's path and the catalog.
/// The folder, the files written into it (name and format), the catalog's path and the catalog.
type Imported = (PathBuf, Vec<(String, Format)>, PathBuf, Catalog);

fn imported(prefix: &str) -> anyhow::Result<Imported> {
    let dir = pg_testkit::run_folder("scratch_rate", prefix)?;
    std::fs::create_dir_all(dir.join("images"))?;
    let files = make_files(&dir)?;
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES)?;
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images"))?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl)?;
    Ok((dir, files, catalog_path, c))
}

/// The thumbnail cache a writer retags, leaked as the app's is, and the check that it reported nothing.
fn thumbs(dir: &Path, c: &Catalog) -> anyhow::Result<(&'static ThumbCache, Quiet<pg_thumbcache::Event>)> {
    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(dir, &c.catalog_id()?, Some(1 << 30), 1, events)?));
    Ok((thumbs, quiet))
}

/// Waits until `done` holds, and fails with `what` after `secs` seconds.
fn wait_until(secs: u64, what: impl Fn() -> String, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(secs), "{}", what());
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The path a rule path names on disk.
fn on_disk(c: &Catalog, path: &pg_model::rules::Path) -> PathBuf {
    pg_platform::names::disk_path(c.rules(), path)
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
fn ratings_and_turns_reach_the_files() {
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        pg_testkit::missing_tool("ExifTool");
        return;
    }
    // Earlier runs' folders go first, and this run's thumbnail cache stays open to the end, so it can't remove its own.
    let dir = pg_testkit::run_folder_with("scratch_rate", "writeback", &["images"]).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    let before = ids(&c).unwrap();
    assert_eq!(before.len(), files.len());

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
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
    let jpeg = images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(jpeg_name)));
    let mut bytes = Cursor::new(Vec::new());
    pattern(64, 48).write_to(&mut bytes, image::ImageFormat::Jpeg).unwrap();
    std::fs::write(&jpeg, bytes.get_ref()).unwrap();
    // The clip from the video crate's tests: 64x48, six frames.
    std::fs::write(
        images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(video_name))),
        include_bytes!("../../video/tests/sample.mp4"),
    )
    .unwrap();

    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
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
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
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

    // A turn of the JPEG is written in place: its pixels are turned, and it keeps its rating. The clip's bytes are
    // left as they were (a video's turn stays in the catalog).
    pg_catalog::writes::queue_rotation(&c, FileId::new(jpeg_id), Turns::new(1)).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let turned = std::fs::read(&jpeg).unwrap();
    let now =
        pg_render::display::decode(&mut Decoder::new().unwrap(), &turned, MinSize::Full, Turns::NONE).unwrap().image;
    assert_eq!((now.width, now.height), (48, 64), "the JPEG is turned in place");
    assert_eq!(Meta::read(&turned, Format::Jpeg, &mut MetaScratch::default()).rating, Some(4));
    let clip = include_bytes!("../../video/tests/sample.mp4");
    assert_eq!(
        std::fs::read(images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(video_name)))).unwrap(),
        clip.as_slice()
    );

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(&images);
    quiet.check();
}

/// A finishing copy whose original is held against writes (another program has it open without sharing on Windows, and
/// the original is read-only on Unix) fails at every try. The queue waits longer between the tries and stops at
/// MAX_ATTEMPTS, but keeps the temp and its record: the original may be part-written, and the temp is the whole new
/// content. The file is still shown as being written, and the error says it may be incomplete and names the temp. Once
/// the file is free, the next start of the writer finishes the copy.
#[test]
fn a_finishing_copy_that_keeps_failing_keeps_its_temp_and_is_finished_later() {
    let (dir, files, catalog_path, c) = imported("giveup").unwrap();
    // The file as the database has it, and the temp a write would have left beside it (writeback's temp_of).
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(&files[1].0, on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();

    // Held against writes: the copy into the original can't open it. Where the permissions don't stop this process
    // (root), there is nothing to test.
    let Some(held) = hold(Path::new(&files[1].0), Access::Write).unwrap() else {
        pg_testkit::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    // The queue's row, and then the status, which the writer sets after it.
    let incomplete = || matches!(writer.status().error, Some(WriteError::CopyIncomplete { .. }));
    wait_until(120, || format!("not at the cap at {} attempts", attempts()), || attempts() >= 5 && incomplete());

    // At the cap: the temp and its record are kept, and the file still reads as being written.
    let row = state::write_row(&c, id).unwrap();
    assert_eq!(row.temp, Some(temp.clone()), "the temp's record is kept");
    assert!(on_disk(&c, &temp).exists(), "the temp is kept on disk");
    let shown_temp = c.rules().display(&temp);
    let error = row.error.unwrap_or_default();
    assert!(error.contains("may be incomplete") && error.contains(&shown_temp), "the queue's error: {error}");
    assert!(
        matches!(&writer.status().error, Some(WriteError::CopyIncomplete { temp, .. }) if *temp == shown_temp),
        "the status: {:?}",
        writer.status().error
    );
    assert!(pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);
    // And it waits at the cap: no more tries until a change or a start.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(attempts(), 5);

    // The file is free, and the writer starts again: it tries the copy again, and finishes it.
    drop(held);
    drop(writer);
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_until(
        60,
        || "the copy is still queued".into(),
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    assert!(!on_disk(&c, &temp).exists(), "the temp is still on disk");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the file holds the rating once the copy is done");
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);
    assert_eq!(writer.status().error, None);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A finishing copy is done with the value its temp carries: a rating set after the temp was recorded stays queued,
/// and is written next. Before, the copy of 3 recorded the file as holding the later 5, and 5 was never written. Needs
/// ExifTool (skipped without it).
#[test]
fn a_rating_set_after_a_temp_was_recorded_is_written_after_the_copy() {
    if std::process::Command::new("exiftool").arg("-ver").output().is_err() {
        pg_testkit::missing_tool("ExifTool");
        return;
    }
    let (dir, files, catalog_path, c) = imported("later").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(&files[1].0, on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    pg_catalog::writes::set_rating(&c, id, Rating::new(5), true).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let bytes = std::fs::read(&files[1].0).unwrap();
    assert_eq!(Meta::read(&bytes, Format::Png, &mut MetaScratch::default()).rating, Some(5), "the later rating");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(5));

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A rating and a turn of one file have their own temps: a turn queued while the rating's copy is stuck at the cap
/// leaves the rating's temp and its record alone, and the rating isn't recorded as written. Before, both used one temp
/// name, and the turn's fresh job deleted the rating's complete temp.
#[test]
fn a_turn_leaves_the_temp_of_a_stuck_rating_alone() {
    let (dir, files, catalog_path, c) = imported("kinds").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let rating_temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    let turns_temp = pg_writeback::temp_of(&db_path, WriteKind::Turns);
    assert_ne!(rating_temp, turns_temp);
    std::fs::copy(&files[1].0, on_disk(&c, &rating_temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &rating_temp).unwrap();
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();
    let Some(held) = hold(Path::new(&files[1].0), Access::Write).unwrap() else {
        pg_testkit::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let attempts = |kind: &str| state::write_row_of(&c, id, kind).unwrap().attempts;
    wait_until(
        120,
        || format!("rating attempts {}, turn attempts {}", attempts("rating"), attempts("rotation")),
        || attempts("rating") >= 5 && attempts("rotation") >= 1,
    );
    assert_eq!(state::write_row_of(&c, id, "rating").unwrap().temp, Some(rating_temp.clone()));
    assert!(on_disk(&c, &rating_temp).exists(), "the rating's temp was deleted");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), None, "the rating was recorded though it isn't in the file");
    assert!(!on_disk(&c, &turns_temp).exists(), "the turn's failed attempt left its temp");

    drop(held);
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A fresh write whose original can't be opened for the copy leaves no temp and no record: the original was not
/// touched, so the next attempt starts from the file, and the file isn't shown as being written.
#[test]
fn a_fresh_write_whose_open_fails_leaves_no_temp_and_no_record() {
    let (dir, files, catalog_path, c) = imported("refused").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Turns);
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();
    let Some(held) = hold(Path::new(&files[1].0), Access::Write).unwrap() else {
        pg_testkit::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    wait_until(60, || format!("{} attempts", attempts()), || attempts() >= 1);
    let row = state::write_row(&c, id).unwrap();
    assert_eq!(row.temp, None, "a temp was recorded though the copy never started");
    assert!(!on_disk(&c, &temp).exists(), "the temp is left on disk");
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);
    assert!(matches!(writer.status().error, Some(WriteError::Failed(_))), "{:?}", writer.status().error);

    drop(held);
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A rating for a file in a folder whose name ends in a dot (`Acme Inc.`, which NTFS holds and the plain path drops
/// the dot of) is written into that file, ExifTool's step included, and the file in `Acme Inc`, the folder the plain
/// path would reach, is left as it was.
#[cfg(windows)]
#[test]
fn a_write_into_a_folder_ending_in_a_dot_reaches_that_folder_and_no_other() {
    use pg_platform::native::verbatim;
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_rate", "dotted").unwrap()).unwrap();
    let (dotted, sibling) = (dir.join("images").join("Acme Inc."), dir.join("images").join("Acme Inc"));
    std::fs::create_dir_all(verbatim(&dotted)).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let mut jpeg = Cursor::new(Vec::new());
    pattern(64, 48).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
    let jpeg = jpeg.into_inner();
    std::fs::write(verbatim(&dotted.join("a.jpg")), &jpeg).unwrap();
    std::fs::write(sibling.join("a.jpg"), &jpeg).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &Control::new()).unwrap();
    // The scan lists both folders, each by its own name.
    let files = state::present_files(&c).unwrap();
    assert_eq!(files.len(), 2, "{files:?}");
    let in_dotted = |id: FileId| {
        let path = c.file_path(id).unwrap().unwrap();
        path.parts.iter().rev().nth(1).is_some_and(|folder| folder.display().ends_with('.'))
    };
    let id = files.iter().map(|(id, _)| *id).find(|&id| in_dotted(id)).expect("the file in the dotted folder");
    pg_catalog::writes::set_rating(&c, id, Rating::new(4), true).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let written = std::fs::read(verbatim(&dotted.join("a.jpg"))).unwrap();
    assert_eq!(Meta::read(&written, Format::Jpeg, &mut MetaScratch::default()).rating, Some(4), "the dotted file");
    assert_eq!(std::fs::read(sibling.join("a.jpg")).unwrap(), jpeg, "the other folder's file changed");
    for folder in [verbatim(&dotted), sibling.clone()] {
        assert!(!folder.join(".a.jpg.rating.pg-write").exists(), "a temp in {}", folder.display());
    }

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(verbatim(&dir.join("images")));
    quiet.check();
}

/// Another program saves the file after the writer read it and before the copy: the attempt fails (the original, as
/// opened for the copy, isn't as it was read), and the next one reads the file again, so the turn is made on that
/// save instead of overwriting it with the turn of the old content.
#[test]
fn a_file_saved_between_the_read_and_the_copy_is_read_again() {
    let (dir, files, catalog_path, c) = imported("saved").unwrap();
    let id = state::file_id(&c, "b.png").unwrap();
    // The other program's save: a smaller image, so its size differs whatever the clock's resolution.
    let mut saved = Cursor::new(Vec::new());
    pattern(32, 16).write_to(&mut saved, image::ImageFormat::Png).unwrap();
    let saved: &'static [u8] = Box::leak(saved.into_inner().into_boxed_slice());
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::SaveBeforeCopy { file: id, with: saved };
    let writer = Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault).unwrap();
    writer.wake();
    wait_until(60, || "the turn is still queued".into(), || state::writes_queued(&c, id).unwrap() == 0);
    let now = shown(&files[1].0).unwrap();
    assert_eq!((now.width, now.height), (16, 32), "the turn of the other program's save");

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A recorded temp that is gone (deleted after its copy by an earlier build, which deleted it before recording the
/// write): the copy is taken as done, and the write is recorded with the value the temp carried.
#[test]
fn a_finishing_copy_whose_temp_is_gone_completes() {
    let (dir, _files, catalog_path, c) = imported("missingtemp").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    assert!(!on_disk(&c, &temp).exists());

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_until(
        60,
        || "the copy is still queued".into(),
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    assert_eq!(writer.status().error, None);
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3));
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A read-only original on a drive that is there is a failure counted like any other, up to the cap. Before, a
/// refused open was taken as an offline drive: never counted, and retried for ever.
#[test]
fn a_read_only_original_is_counted_up_to_the_cap() {
    let (dir, files, catalog_path, c) = imported("readonly").unwrap();
    let id = state::file_id(&c, "b.png").unwrap();
    let path = Path::new(&files[1].0);
    let before = std::fs::metadata(path).unwrap().permissions();
    let mut read_only = before.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(path, read_only).unwrap();
    if std::fs::OpenOptions::new().write(true).open(path).is_ok() {
        std::fs::set_permissions(path, before).unwrap();
        pg_testkit::skip("the permissions don't stop this process from writing (root)");
        return;
    }
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    let cap = pg_catalog::writes::MAX_ATTEMPTS;
    wait_until(120, || format!("{} attempts", attempts()), || attempts() >= cap);
    let row = state::write_row(&c, id).unwrap();
    assert!(row.error.is_some() && row.temp.is_none(), "{:?}", row.error);
    assert_eq!(pg_catalog::writes::left_to_write(&c).unwrap(), 0, "nothing left to try");

    drop(writer);
    std::fs::set_permissions(path, before).unwrap();
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A rating queued for a file that can't hold one leaves the queue as done, with no error: a BMP (known from its
/// analysis), and a `.jpg` that is no image the app reads (its analysis failed, and its bytes are no format). Before,
/// the second failed every attempt with "unknown format", which the status kept showing.
#[test]
fn a_rating_for_a_file_that_cant_hold_one_leaves_the_queue() {
    let dir = pg_testkit::run_folder("scratch_rate", "cantholds").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let junk = dir.join("images").join("g.jpg");
    std::fs::write(&junk, b"not an image at all").unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();
    let (bmp, jpg) = (state::file_id(&c, "f.bmp").unwrap(), state::file_id(&c, "g.jpg").unwrap());
    for id in [bmp, jpg] {
        state::queue_write(&c, id, "rating", 3).unwrap();
    }

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let left = || state::writes_queued(&c, bmp).unwrap() + state::writes_queued(&c, jpg).unwrap();
    wait_until(60, || format!("{} still queued", left()), || left() == 0);
    assert_eq!(writer.status().error, None);
    assert_eq!(std::fs::read(&junk).unwrap(), b"not an image at all");
    let bmp_bytes = std::fs::read(&files[5].0).unwrap();
    assert_eq!(Meta::read(&bmp_bytes, Format::Bmp, &mut MetaScratch::default()).rating, None);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A write for a file the catalog marks missing is dropped, and counted for the app's note (`WriteStatus::dropped`);
/// a file deleted outside the app while the catalog still lists it is tried, and each failure counted (it isn't waited
/// on as offline). Once that file is marked missing too, its write is dropped, and the failure the status showed for
/// it goes with it.
#[test]
fn a_write_for_a_file_that_is_gone_is_dropped_and_a_deleted_one_is_counted() {
    let (dir, files, catalog_path, c) = imported("gone").unwrap();
    // a.jpg: a scan has marked it missing, so its write is dropped. b.png was deleted outside the app while the
    // catalog still lists it: its write is tried, and each failure is counted (it isn't waited on as offline).
    let (marked, deleted) = (state::file_id(&c, "a.jpg").unwrap(), state::file_id(&c, "b.png").unwrap());
    state::mark_missing(&c, marked).unwrap();
    std::fs::remove_file(&files[1].0).unwrap();
    for file in [marked, deleted] {
        state::queue_write(&c, file, "rating", 3).unwrap();
    }

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    let queued = || state::writes_queued(&c, marked).unwrap();
    let attempts = || state::write_row(&c, deleted).unwrap().attempts;
    // The queue, and then the status, which the writer sets after it.
    let noted = || {
        let s = writer.status();
        s.dropped == 1 && s.error_file == Some(deleted)
    };
    wait_until(
        60,
        || format!("queued {}, deleted file's attempts {}", queued(), attempts()),
        || queued() == 0 && attempts() >= 2 && noted(),
    );
    let status = writer.status();
    assert_eq!((status.dropped, status.last_dropped), (1, Some(WriteKind::Rating)));
    assert!(status.error.is_some(), "the deleted file's failure is in the status line");
    assert_eq!(status.error_file, Some(deleted));

    // The deleted file is marked missing too: its write is dropped, and its failure is no longer shown.
    state::mark_missing(&c, deleted).unwrap();
    writer.wake();
    wait_until(
        60,
        || "the deleted file's write is still queued".into(),
        || state::writes_queued(&c, deleted).unwrap() == 0 && writer.status().dropped == 2,
    );
    let status = writer.status();
    assert_eq!(status.dropped, 2);
    assert_eq!((status.error, status.error_file), (None, None), "the failure of a dropped write is still shown");

    drop(writer);
    drop(c);
    quiet.check();
}

/// A copy that may have started, for a file the catalog marks missing, is not dropped: its temp is the only whole copy of
/// the new content, so the temp and the row are kept until the file is listed again, and the status says so.
#[test]
fn a_copy_that_may_have_started_is_kept_while_its_file_is_missing() {
    let (dir, files, catalog_path, c) = imported("missingcopy").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(&files[1].0, on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    state::mark_missing(&c, id).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    // The status is set after the job's failure is recorded in the queue.
    wait_until(60, || "the copy's failure is not shown".into(), || writer.status().error_file == Some(id));
    assert_eq!(writer.status().dropped, 0, "kept, not dropped");
    assert_eq!(state::writes_queued(&c, id).unwrap(), 1, "the row is kept");
    assert_eq!(state::write_row(&c, id).unwrap().temp, Some(temp.clone()), "the temp's record is kept");
    assert!(on_disk(&c, &temp).exists(), "the temp is kept on disk");
    let error = state::write_row(&c, id).unwrap().error.unwrap_or_default();
    assert!(error.contains("no longer in the library") && error.contains(&c.rules().display(&temp)), "{error}");

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A copy that fails once its temp is recorded (every copy of the file does, as a disk full would: `Fault::FailCopy`) is
/// a copy that may have started. A fresh job's failure at the cap is `CopyIncomplete` and names the temp, and the temp
/// and its record are kept, the file reads as being written. Before, the fresh job's own failure had no temp to name,
/// so the cap gave `Failed`. Turns, so that no ExifTool is needed.
#[test]
fn a_copy_failing_after_its_temp_is_recorded_is_incomplete_at_the_cap() {
    let (dir, _files, catalog_path, c) = imported("failcopy").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Turns);
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::FailCopy(id);
    let writer = Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    // The queue's row, and then the status, which the writer sets after it.
    let incomplete = || matches!(writer.status().error, Some(WriteError::CopyIncomplete { .. }));
    wait_until(120, || format!("not at the cap at {} attempts", attempts()), || attempts() >= 5 && incomplete());

    let row = state::write_row(&c, id).unwrap();
    assert_eq!(row.temp, Some(temp.clone()), "the temp's record is kept");
    assert!(on_disk(&c, &temp).exists(), "the temp is kept on disk");
    let shown_temp = c.rules().display(&temp);
    let error = row.error.unwrap_or_default();
    assert!(error.contains("may be incomplete") && error.contains(&shown_temp), "the queue's error: {error}");
    assert!(
        matches!(&writer.status().error, Some(WriteError::CopyIncomplete { temp, .. }) if *temp == shown_temp),
        "the status: {:?}",
        writer.status().error
    );
    assert!(pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A job whose row leaves the queue after the writer read the file and opened it for the copy (here a change that cancels
/// the write removes it, `Fault::ForgetBeforeRecord`) is not copied: the original is as it was, and no temp is left.
#[test]
fn a_job_cancelled_before_its_temp_is_recorded_leaves_the_original_as_it_was() {
    let (dir, files, catalog_path, c) = imported("cancelled").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Turns);
    let before = std::fs::read(&files[1].0).unwrap();
    pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::ForgetBeforeRecord(id);
    let writer = Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault).unwrap();
    writer.wake();
    // The row is gone at the record, and the temp (made before it) goes right after: both are done when the temp is.
    wait_until(
        60,
        || "the cancelled job's temp is still on disk".into(),
        || state::writes_queued(&c, id).unwrap() == 0 && !on_disk(&c, &temp).exists(),
    );
    assert_eq!(std::fs::read(&files[1].0).unwrap(), before, "the original is as it was");
    assert!(!pg_catalog::browse::read::file_state(&c, id).unwrap().unwrap().writing);
    assert_eq!(writer.status().error, None);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

#[test]
fn a_finished_rotation_copy_turns_the_analysis_back_as_a_normal_write_does() {
    // Its own folder, as the other tests here: each run_folder call gets a folder of its own.
    let dir = pg_testkit::run_folder("scratch_rate", "finish").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // A turn of b.png whose copy was complete when the app stopped: its temp is beside it, and the analysis still
    // shows the turn pending (rotation 1).
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Turns);
    std::fs::copy(&files[1].0, pg_platform::names::disk_path(c.rules(), &temp)).unwrap();
    state::set_analysis_rotation(&c, id, 1).unwrap();
    state::queue_finished_copy(&c, id, "rotation", 1, &temp).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
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

/// A job that panics (as a damaged file the edit can't take would) is that job's failure, recorded and counted as any
/// other: the writer goes on with the other jobs and stays running. Before, the panic ended the writer thread with
/// nothing recorded, and every later start died on the same job.
#[test]
fn a_job_that_panics_fails_alone_and_the_writer_goes_on() {
    // Its own folder, as the other tests here: each run_folder call gets a folder of its own.
    let dir = pg_testkit::run_folder("scratch_rate", "poisoned").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();
    // Turns: a PNG's is rewritten without ExifTool.
    let (poisoned, healthy) = (state::file_id(&c, "a.jpg").unwrap(), state::file_id(&c, "b.png").unwrap());
    for id in [poisoned, healthy] {
        pg_catalog::writes::queue_rotation(&c, id, Turns::new(1)).unwrap();
    }

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
    let writer =
        Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), Fault::PanicInProcess(poisoned))
            .unwrap();
    writer.wake();

    let start = Instant::now();
    loop {
        let healthy_left = state::writes_queued(&c, healthy).unwrap();
        let attempts = state::write_row(&c, poisoned).unwrap().attempts;
        if healthy_left == 0 && attempts >= 1 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "healthy turn queued {healthy_left}, attempts {attempts}");
        std::thread::sleep(Duration::from_millis(200));
    }
    let error = state::write_row(&c, poisoned).unwrap().error;
    assert!(error.as_deref().is_some_and(|e| e.contains("a panic in the job of file")), "the failure: {error:?}");
    assert!(writer.is_running(), "the panic ended the writer");
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A panic outside any job (where the writer reads the queue) is not caught: nothing would record it, and a writer
/// kept running would meet it again at once. The thread ends, and says so, with the write still queued, so `gallery
/// write` stops waiting for it.
#[test]
fn a_panic_outside_a_job_ends_the_writer_and_says_so() {
    let dir = pg_testkit::run_folder("scratch_rate", "drainpanic").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_catalog::writes::queue_rotation(&c, state::file_id(&c, "b.png").unwrap(), Turns::new(1)).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
    let writer =
        Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), Fault::PanicInDrain).unwrap();
    let start = Instant::now();
    while writer.is_running() {
        assert!(start.elapsed() < Duration::from_secs(60), "the writer is still running");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(pg_catalog::writes::left_to_write(&c).unwrap(), 1, "the write is still queued");
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A finishing copy queued while its temp was text (the catalog before temps were kept as bytes) is carried over by the
/// upgrade and finished: the original takes the temp's content, the rating is applied, and the temp is gone.
#[test]
fn a_finishing_copy_carried_from_a_text_temp_is_finished() {
    let dir = pg_testkit::run_folder_with("scratch_writeback", "carried-temp", &["images"]).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &Options::new(pg_platform::priority::Priority::Normal), false, &ctl).unwrap();

    // The temp a rating of b.png left: it holds a.jpg's bytes, so the copy into b.png is visible.
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    let temp_text = String::from_utf8(c.rules().join(&temp)).unwrap();
    let copied = std::fs::read(&files[0].0).unwrap();
    std::fs::write(pg_platform::names::disk_path(c.rules(), &temp), &copied).unwrap();
    drop(c);

    // The catalog as it was before, with the temp as text.
    state::set_text_temp_layout(&catalog_path, id, &temp_text).unwrap();

    // The upgrade carries the row (its temp as bytes), and the writer finishes the copy.
    let c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    assert!(c.rebuilt().is_some(), "the text layout is moved aside");
    assert_eq!(state::write_row(&c, id).unwrap().temp, Some(temp.clone()));
    let (events, quiet) = Quiet::bus();
    let thumbs: &'static ThumbCache =
        Box::leak(Box::new(ThumbCache::open(&dir, &c.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap()));
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16))).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    assert_eq!(std::fs::read(&files[1].0).unwrap(), copied, "the copy went into the original");
    assert!(!pg_platform::names::disk_path(c.rules(), &temp).exists(), "the temp is still on disk");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the file holds the rating once the copy is done");

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}
