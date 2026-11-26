//! Write-back on real files: ratings into each format, lossless turns, file identity kept, no re-import after a
//! metadata write, and a rating changed by another program adopted. Needs ExifTool (skipped without it).
//!
//! Files are made in a scratch folder (`PG_TEST_DIR`, or the temp folder's `scratch_rate`), never in a dataset.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_codec::decode::{Decoder, MinSize};
use pg_codec::fixtures::Encoding;
use pg_codec::fixtures::orient;
use pg_codec::pixels::Rgb;
use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_jobs::pipeline::Options;
use pg_jobs_testkit::{Call, Quiet, RecordingSink};
use pg_metadata::{Meta, MetaScratch};
use pg_model::format::Format;
use pg_model::ids::FileId;
use pg_model::notice::{Notice, Trigger};
use pg_model::rating::Rating;
use pg_model::turns::Turns;
use pg_model::write::WriteError;
use pg_model::write::WriteKind;
use pg_platform::testing::held::{Access, hold};
use pg_thumbcache::ThumbCache;
use pg_writeback::Writer;
use pg_writeback::fault::{Fault, PauseGate};

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A `w` × `h` test picture, encoded as `encoding`: its channels step across it, so a crop or a turn moves pixels that
/// differ from each other.
fn pattern(w: u32, h: u32, encoding: Encoding) -> Result<Vec<u8>> {
    let rgb = pg_testkit::rgb_pattern(usize::try_from(w)?, usize::try_from(h)?);
    pg_codec::fixtures::encode(w, h, &rgb, encoding).map_err(Into::into)
}

/// One file per format, sized so the JPEG transform is perfect (multiples of 16).
fn make_files(dir: &Path) -> Result<Vec<(String, Format)>> {
    let mut out = Vec::new();
    for (name, encoding) in [
        ("a.jpg", Encoding::Jpeg),
        ("b.png", Encoding::Png),
        ("c.webp", Encoding::WebP),
        ("d.tif", Encoding::Tiff),
        ("e.gif", Encoding::Gif),
        ("f.bmp", Encoding::Bmp),
    ] {
        let bytes = pattern(64, 48, encoding)?;
        let path = dir.join("images").join(name);
        std::fs::write(&path, &bytes)?;
        let sniffed = Format::sniff(&bytes).ok_or_else(|| format!("{name}: not a format the codec knows"))?;
        out.push((path.to_string_lossy().into_owned(), sniffed));
    }
    Ok(out)
}

fn shown(path: &str) -> Result<Rgb> {
    Ok(pg_render::display::decode(&mut Decoder::new()?, &std::fs::read(path)?, MinSize::Full, Turns::NONE)?.image)
}

fn ids(c: &Catalog) -> Result<Vec<(i64, String)>> {
    Ok(state::present_files(c)?.into_iter().map(|(id, name)| (id.get(), name)).collect())
}

/// A library of [`make_files`] in a folder of its own (`prefix`; each run_folder call gets a folder of its own, and these
/// tests run beside each other), scanned and imported (the type below says what it holds).
/// The folder, the files written into it (name and format), the catalog's path and the catalog.
type Imported = (PathBuf, Vec<(String, Format)>, PathBuf, Catalog);

fn imported(prefix: &str) -> Result<Imported> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    std::fs::create_dir_all(dir.join("images"))?;
    let files = make_files(&dir)?;
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES)?;
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images"))?;
    c.add_root(&root)?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    pg_import::import(
        &mut c,
        &Options::new(pg_platform::priority::Priority::Normal),
        &pg_catalog::VolumeSet::All,
        &ctl,
    )?;
    Ok((dir, files, catalog_path, c))
}

/// The thumbnail cache a writer retags, shared with the writer as the app shares its own, and the check that it reported
/// nothing.
fn thumbs(dir: &Path, c: &Catalog) -> Result<(Arc<ThumbCache>, Quiet<pg_thumbcache::Event>)> {
    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(ThumbCache::open(
        &dir.join("thumbs.sqlite"),
        &c.catalog_id()?,
        pg_model::cache::ThumbCap::from_bytes(1 << 30),
        pg_model::versions::THUMBS_VERSION,
        events,
    )?);
    Ok((thumbs, quiet))
}

/// The path a rule path names on disk.
fn on_disk(c: &Catalog, path: &pg_model::rules::Path) -> PathBuf {
    pg_platform::names::disk_path(c.rules(), path)
}

fn wait_for_queue(c: &Catalog) -> Result<()> {
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
    if !pg_platform::testing::require_tool("ExifTool", "exiftool", "-ver") {
        return;
    }
    // Earlier runs' folders go first, and this run's thumbnail cache stays open to the end, so it can't remove its own.
    let dir = pg_testkit::run_folder_with("scratch_library", "writeback", &["images"]).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    // The placeholders the grid shows, made from the originals (no cache): their turns are what the turns below check.
    pg_thumbs::placeholders::run_while(
        &mut c,
        &ThumbCache::disabled(),
        &opts,
        &pg_catalog::VolumeSet::All,
        &|| true,
        &pg_thumbs::new_maker,
        &ctl,
    )
    .unwrap();
    let before = ids(&c).unwrap();
    assert_eq!(before.len(), files.len());

    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(
        ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap(),
    );
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();

    // Ratings: every format but BMP gets it in the file; nothing needs re-importing afterwards.
    for (n, (id, _)) in before.iter().enumerate() {
        let rating = Rating::new(u8::try_from(n % 5 + 1).unwrap());
        c.set_rating(FileId::new(*id), rating, true).unwrap();
    }
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    for (n, (path, format)) in files.iter().enumerate() {
        let expected = if *format == Format::Bmp { None } else { Some(u8::try_from(n % 5 + 1).unwrap()) };
        assert_eq!(
            Meta::read(&std::fs::read(path).unwrap(), *format, &mut MetaScratch::default()).rating.map(Rating::get),
            expected,
            "{path}"
        );
    }
    assert_eq!(c.status().unwrap().pending, 0, "a metadata write must not trigger a re-import");
    // Same files, same identities (written in place).
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    assert_eq!(c.status().unwrap().missing, 0);

    // A turn: the files show turned; the analysis is redone with the new size.
    let originals: Vec<Rgb> = files.iter().map(|(p, _)| shown(p).unwrap()).collect();
    for (id, _) in &before {
        c.queue_rotation(FileId::new(*id), Turns::new(1)).unwrap();
    }
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    for ((path, format), original) in files.iter().zip(originals) {
        let turned = orient(original, pg_model::orientation::Orientation::new(6).unwrap());
        let rotation = state::file_rotation(&c, &Path::new(path).file_name().unwrap().to_string_lossy()).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let now =
            pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, rotation).unwrap().image;
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
        before.iter().map(|(id, _)| c.placeholder(FileId::new(*id)).unwrap().unwrap().shown_turns.get()).collect()
    };
    assert!(placeholder_turns(&c).iter().all(|&t| t == 1), "placeholders after the turn: {:?}", placeholder_turns(&c));
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    assert!(dims(&c).iter().all(|&d| d == (48, 64)), "re-imported upright sizes: {:?}", dims(&c));
    // Until the Placeholders pass makes them again, the placeholders show the pixels they were made from, turned.
    assert!(placeholder_turns(&c).iter().all(|&t| t == 1), "before the pass: {:?}", placeholder_turns(&c));
    pg_thumbs::placeholders::run_while(
        &mut c,
        &ThumbCache::disabled(),
        &opts,
        &pg_catalog::VolumeSet::All,
        &|| true,
        &pg_thumbs::new_maker,
        &ctl,
    )
    .unwrap();
    assert!(placeholder_turns(&c).iter().all(|&t| t == 0), "re-made placeholders: {:?}", placeholder_turns(&c));

    // Another program rates the JPEG: the next scan and import adopt it.
    let jpg = &files[0].0;
    let out = std::process::Command::new("exiftool")
        .args(["-overwrite_original_in_place", "-XMP:Rating=2", jpg])
        .output()
        .unwrap();
    assert!(out.status.success());
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    assert_eq!(state::rating_of(&c, FileId::new(before[0].0)).unwrap(), Some(2));

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A JPEG that Windows rated (4 stars in XMP and a percent of 75 in EXIF) takes a new rating through the queue: the write
/// is accepted (no write error), and the file reads back the rating written, not the stale percent. Needs ExifTool;
/// skipped without it.
#[test]
fn a_rating_reaches_a_jpeg_that_holds_a_stale_percent() {
    if !pg_platform::testing::require_tool("ExifTool", "exiftool", "-ver") {
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_library", "stale", &["images"]).unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let path = dir.join("images").join("stale.jpg");
    std::fs::write(&path, pattern(64, 48, Encoding::Jpeg).unwrap()).unwrap();
    let out = std::process::Command::new("exiftool")
        .args(["-overwrite_original_in_place", "-XMP:Rating=4", "-EXIF:RatingPercent=75"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let (id, _) = ids(&c).unwrap().into_iter().next().unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    c.set_rating(FileId::new(id), Rating::new(2), true).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let written = std::fs::read(&path).unwrap();
    assert_eq!(Meta::read(&written, Format::Jpeg, &mut MetaScratch::default()).rating.map(Rating::get), Some(2));

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
    if !pg_platform::testing::require_tool("ExifTool", "exiftool", "-ver") {
        return;
    }
    let dir = pg_testkit::run_folder_with("scratch_library", "odd-names", &["images"]).unwrap();
    let (jpeg_name, video_name): (&[u8], &[u8]) = if cfg!(windows) {
        (b"odd\xED\xA0\x80.jpg", b"clip\xED\xA0\x80.mp4")
    } else {
        (b"odd\xFF.jpg", b"clip\xFF.mp4")
    };
    let images = dir.join("images");
    let jpeg = images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(jpeg_name)));
    std::fs::write(&jpeg, pattern(64, 48, Encoding::Jpeg).unwrap()).unwrap();
    // The clip from the video crate's tests: 64x48, six frames.
    std::fs::write(
        images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(video_name))),
        pg_testkit::VIDEO_SAMPLE,
    )
    .unwrap();

    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    let before = ids(&c).unwrap();
    assert_eq!(before.len(), 2, "both imported: {before:?}");
    let jpeg_id = before.iter().find(|(_, name)| name.ends_with(".jpg")).expect("the JPEG").0;

    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(
        ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap(),
    );
    // The thumbnail pass makes both: the JPEG's, and the clip's (read by the video reader from its own bytes).
    let sixty_four = pg_model::size::ThumbHeight::new(64).expect("a height in range");
    let made = pg_thumbs::run(&thumbs, &catalog_path, sixty_four, &opts, &ctl).unwrap();
    assert_eq!((made.made, made.failed), (2, 0), "thumbnails of both files");
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    c.set_rating(FileId::new(jpeg_id), Rating::new(4), true).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    assert_eq!(
        Meta::read(&std::fs::read(&jpeg).unwrap(), Format::Jpeg, &mut MetaScratch::default()).rating.map(Rating::get),
        Some(4)
    );
    // Written in place: the same identities, and no other file beside them.
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    assert_eq!(ids(&c).unwrap(), before);
    assert_eq!(std::fs::read_dir(&images).unwrap().count(), 2);

    // A turn of the JPEG is written in place: its pixels are turned, and it keeps its rating. The clip's bytes are
    // left as they were (a video's turn stays in the catalog).
    c.queue_rotation(FileId::new(jpeg_id), Turns::new(1)).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let turned = std::fs::read(&jpeg).unwrap();
    let now =
        pg_render::display::decode(&mut Decoder::new().unwrap(), &turned, MinSize::Full, Turns::NONE).unwrap().image;
    assert_eq!((now.width, now.height), (48, 64), "the JPEG is turned in place");
    assert_eq!(Meta::read(&turned, Format::Jpeg, &mut MetaScratch::default()).rating.map(Rating::get), Some(4));
    let clip = pg_testkit::VIDEO_SAMPLE;
    assert_eq!(
        std::fs::read(images.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(video_name)))).unwrap(),
        clip
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
        pg_platform::testing::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    // The queue's row, and then the status, which the writer sets after it.
    let incomplete = || matches!(writer.status().error, Some(WriteError::CopyIncomplete { .. }));
    pg_testkit::wait_until(
        Duration::from_secs(120),
        || format!("not at the cap at {} attempts", attempts()),
        || attempts() >= pg_catalog::queue::MAX_ATTEMPTS && incomplete(),
    );

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
    assert!(c.file_state(id).unwrap().unwrap().writing);
    // And it waits at the cap: no more tries until a change or a start.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(attempts(), pg_catalog::queue::MAX_ATTEMPTS);

    // The file is free, and the writer starts again: it tries the copy again, and finishes it.
    drop(held);
    drop(writer);
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the copy is still queued",
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    assert!(!on_disk(&c, &temp).exists(), "the temp is still on disk");
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the file holds the rating once the copy is done");
    assert!(!c.file_state(id).unwrap().unwrap().writing);
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
    if !pg_platform::testing::require_tool("ExifTool", "exiftool", "-ver") {
        return;
    }
    let (dir, files, catalog_path, c) = imported("later").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(&files[1].0, on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    c.set_rating(id, Rating::new(5), true).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let bytes = std::fs::read(&files[1].0).unwrap();
    assert_eq!(
        Meta::read(&bytes, Format::Png, &mut MetaScratch::default()).rating.map(Rating::get),
        Some(5),
        "the later rating"
    );
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
    let turns_temp = pg_writeback::temp_of(&db_path, WriteKind::Rotation);
    assert_ne!(rating_temp, turns_temp);
    std::fs::copy(&files[1].0, on_disk(&c, &rating_temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &rating_temp).unwrap();
    c.queue_rotation(id, Turns::new(1)).unwrap();
    let Some(held) = hold(Path::new(&files[1].0), Access::Write).unwrap() else {
        pg_platform::testing::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    let attempts = |kind: &str| state::write_row_of(&c, id, kind).unwrap().attempts;
    pg_testkit::wait_until(
        Duration::from_secs(120),
        || format!("rating attempts {}, turn attempts {}", attempts("rating"), attempts("rotation")),
        || attempts("rating") >= pg_catalog::queue::MAX_ATTEMPTS && attempts("rotation") >= 1,
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
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rotation);
    c.queue_rotation(id, Turns::new(1)).unwrap();
    let Some(held) = hold(Path::new(&files[1].0), Access::Write).unwrap() else {
        pg_platform::testing::skip("the permissions don't stop this process from writing (root)");
        return;
    };

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    pg_testkit::wait_until(Duration::from_secs(60), || format!("{} attempts", attempts()), || attempts() >= 1);
    let row = state::write_row(&c, id).unwrap();
    assert_eq!(row.temp, None, "a temp was recorded though the copy never started");
    assert!(!on_disk(&c, &temp).exists(), "the temp is left on disk");
    assert!(!c.file_state(id).unwrap().unwrap().writing);
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
    let dir = std::path::absolute(pg_testkit::run_folder("scratch_library", "dotted").unwrap()).unwrap();
    let (dotted, sibling) = (dir.join("images").join("Acme Inc."), dir.join("images").join("Acme Inc"));
    std::fs::create_dir_all(verbatim(&dotted)).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let jpeg = pattern(64, 48, Encoding::Jpeg).unwrap();
    std::fs::write(verbatim(&dotted.join("a.jpg")), &jpeg).unwrap();
    std::fs::write(sibling.join("a.jpg"), &jpeg).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &Control::new()).unwrap();
    // The scan lists both folders, each by its own name.
    let files = state::present_files(&c).unwrap();
    assert_eq!(files.len(), 2, "{files:?}");
    let in_dotted = |id: FileId| {
        let path = c.file_path(id).unwrap().unwrap();
        path.parts.iter().rev().nth(1).is_some_and(|folder| folder.display().ends_with('.'))
    };
    let id = files.iter().map(|(id, _)| *id).find(|&id| in_dotted(id)).expect("the file in the dotted folder");
    c.set_rating(id, Rating::new(4), true).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let written = std::fs::read(verbatim(&dotted.join("a.jpg"))).unwrap();
    assert_eq!(
        Meta::read(&written, Format::Jpeg, &mut MetaScratch::default()).rating.map(Rating::get),
        Some(4),
        "the dotted file"
    );
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
    let saved: &'static [u8] = Box::leak(pattern(32, 16, Encoding::Png).unwrap().into_boxed_slice());
    c.queue_rotation(id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::SaveBeforeCopy { file: id, with: saved };
    let writer =
        Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault, RecordingSink::new())
            .unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the turn is still queued",
        || state::writes_queued(&c, id).unwrap() == 0,
    );
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
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the copy is still queued",
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    assert_eq!(writer.status().error, None);
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3));
    assert!(!c.file_state(id).unwrap().unwrap().writing);

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
        pg_platform::testing::skip("the permissions don't stop this process from writing (root)");
        return;
    }
    c.queue_rotation(id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    let cap = pg_catalog::queue::MAX_ATTEMPTS;
    pg_testkit::wait_until(Duration::from_secs(120), || format!("{} attempts", attempts()), || attempts() >= cap);
    let row = state::write_row(&c, id).unwrap();
    assert!(row.error.is_some() && row.temp.is_none(), "{:?}", row.error);
    assert_eq!(c.left_to_write().unwrap(), 0, "nothing left to try");

    drop(writer);
    std::fs::set_permissions(path, before).unwrap();
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A rating queued for a file that can't hold one leaves the queue as done, with no error: a BMP (known from its
/// analysis), and a `.jpg` that is no image the app reads (its analysis failed, and its bytes are no format). Before,
/// the second failed every attempt with "unknown image format", which the status kept showing.
#[test]
fn a_rating_for_a_file_that_cant_hold_one_leaves_the_queue() {
    let dir = pg_testkit::run_folder("scratch_library", "cantholds").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let junk = dir.join("images").join("g.jpg");
    std::fs::write(&junk, b"not an image at all").unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(
        &mut c,
        &Options::new(pg_platform::priority::Priority::Normal),
        &pg_catalog::VolumeSet::All,
        &ctl,
    )
    .unwrap();
    let (bmp, jpg) = (state::file_id(&c, "f.bmp").unwrap(), state::file_id(&c, "g.jpg").unwrap());
    for id in [bmp, jpg] {
        state::queue_write(&c, id, "rating", 3).unwrap();
    }

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    let left = || state::writes_queued(&c, bmp).unwrap() + state::writes_queued(&c, jpg).unwrap();
    pg_testkit::wait_until(Duration::from_secs(60), || format!("{} still queued", left()), || left() == 0);
    assert_eq!(writer.status().error, None);
    assert_eq!(std::fs::read(&junk).unwrap(), b"not an image at all");
    let bmp_bytes = std::fs::read(&files[5].0).unwrap();
    assert_eq!(Meta::read(&bmp_bytes, Format::Bmp, &mut MetaScratch::default()).rating.map(Rating::get), None);

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
    let sink = RecordingSink::new();
    let writer = Writer::start(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), sink.clone()).unwrap();
    writer.wake();
    let queued = || state::writes_queued(&c, marked).unwrap();
    let attempts = || state::write_row(&c, deleted).unwrap().attempts;
    // The queue, and then the status, which the writer sets after it.
    let noted = || {
        let s = writer.status();
        s.dropped == 1 && s.error_file == Some(deleted)
    };
    pg_testkit::wait_until(
        Duration::from_secs(60),
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
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the deleted file's write is still queued",
        || state::writes_queued(&c, deleted).unwrap() == 0 && writer.status().dropped == 2,
    );
    let status = writer.status();
    assert_eq!(status.dropped, 2);
    assert_eq!((status.error, status.error_file), (None, None), "the failure of a dropped write is no longer shown");

    // The writer told it all: each drop (an event, its count with the drain's others), the deleted file's failure
    // once (the same failure again is not news), and its end once its write was dropped.
    drop(writer);
    let told = sink.take();
    let dropped: u64 = told
        .iter()
        .filter_map(|call| match call {
            Call::Post(Notice::WriteDropped { write: WriteKind::Rating, count }) => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(dropped, 2, "{told:?}");
    let failures = told.iter().filter(|call| matches!(call, Call::Post(Notice::WriteFailed { .. }))).count();
    assert!(failures >= 1, "{told:?}");
    let failure_calls: Vec<&Call> = told
        .iter()
        .filter(|call| match call {
            Call::Post(notice) => matches!(notice, Notice::WriteFailed { .. }),
            Call::Resolve(trigger) => *trigger == Trigger::Status,
            Call::Replace(..) => false,
        })
        .collect();
    assert_eq!(failure_calls.last(), Some(&&Call::Resolve(Trigger::Status)), "{told:?}");

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
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    // The status is set after the job's failure is recorded in the queue.
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the copy's failure is not shown",
        || writer.status().error_file == Some(id),
    );
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
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rotation);
    c.queue_rotation(id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::FailCopy(id);
    let writer =
        Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault, RecordingSink::new())
            .unwrap();
    writer.wake();
    let attempts = || state::write_row(&c, id).unwrap().attempts;
    // The queue's row, and then the status, which the writer sets after it.
    let incomplete = || matches!(writer.status().error, Some(WriteError::CopyIncomplete { .. }));
    pg_testkit::wait_until(
        Duration::from_secs(120),
        || format!("not at the cap at {} attempts", attempts()),
        || attempts() >= pg_catalog::queue::MAX_ATTEMPTS && incomplete(),
    );

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
    assert!(c.file_state(id).unwrap().unwrap().writing);

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
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rotation);
    let before = std::fs::read(&files[1].0).unwrap();
    c.queue_rotation(id, Turns::new(1)).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let fault = Fault::ForgetBeforeRecord(id);
    let writer =
        Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), fault, RecordingSink::new())
            .unwrap();
    writer.wake();
    // The row is gone at the record, and the temp (made before it) goes right after: both are done when the temp is.
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the cancelled job's temp is still on disk",
        || state::writes_queued(&c, id).unwrap() == 0 && !on_disk(&c, &temp).exists(),
    );
    assert_eq!(std::fs::read(&files[1].0).unwrap(), before, "the original is as it was");
    assert!(!c.file_state(id).unwrap().unwrap().writing);
    assert_eq!(writer.status().error, None);

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

#[test]
fn a_finished_rotation_copy_turns_the_analysis_back_as_a_normal_write_does() {
    // Its own folder, as the other tests here: each run_folder call gets a folder of its own.
    let dir = pg_testkit::run_folder("scratch_library", "finish").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let files = make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(
        &mut c,
        &Options::new(pg_platform::priority::Priority::Normal),
        &pg_catalog::VolumeSet::All,
        &ctl,
    )
    .unwrap();

    // A turn of b.png whose copy was complete when the app stopped: its temp is beside it, and the analysis still
    // shows the turn pending (rotation 1).
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rotation);
    std::fs::copy(&files[1].0, pg_platform::names::disk_path(c.rules(), &temp)).unwrap();
    state::set_analysis_rotation(&c, id, Turns::new(1)).unwrap();
    state::queue_finished_copy(&c, id, "rotation", 1, &temp).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(
        ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap(),
    );
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
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
    assert_eq!(rotation, Turns::NONE, "the turn is in the file, so the analysis no longer shows it pending");
    quiet.check();
}

/// Forty-two turns of one PNG queued while the writer is making them (a wake every few): all are made, once each, so the
/// file ends turned by 42 mod 4 = 2 quarter turns, and nothing is written twice or lost.
#[test]
fn rapid_turns_of_one_file_are_all_made_once() {
    let (dir, files, catalog_path, c) = imported("rapid-turns").unwrap();
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    let (path, _) = &files[1];
    let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
    let id = state::file_id(&c, &name).unwrap();
    let original = shown(path).unwrap();
    for n in 0..42 {
        c.queue_rotation(id, Turns::new(1)).unwrap();
        if n % 5 == 0 {
            writer.wake();
        }
    }
    writer.wake();
    wait_for_queue(&c).unwrap();
    assert_eq!(writer.status().error, None);
    let turned = orient(original, pg_model::orientation::Orientation::new(3).unwrap());
    let rotation = state::file_rotation(&c, &name).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let now = pg_render::display::decode(&mut Decoder::new().unwrap(), &bytes, MinSize::Full, rotation).unwrap().image;
    assert_eq!((now.width, now.height), (turned.width, turned.height), "{path}");
    assert_eq!(now.pixels, turned.pixels, "{path}: lossless, turned by 2 in all");
    drop(writer);
    drop(c);
    quiet.check();
}

/// A job that panics (as a damaged file the edit can't take would) is that job's failure, recorded and counted as any
/// other: the writer goes on with the other jobs and stays running. Before, the panic ended the writer thread with
/// nothing recorded, and every later start died on the same job.
#[test]
fn a_job_that_panics_fails_alone_and_the_writer_goes_on() {
    // Its own folder, as the other tests here: each run_folder call gets a folder of its own.
    let dir = pg_testkit::run_folder("scratch_library", "poisoned").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(
        &mut c,
        &Options::new(pg_platform::priority::Priority::Normal),
        &pg_catalog::VolumeSet::All,
        &ctl,
    )
    .unwrap();
    // Turns: a PNG's is rewritten without ExifTool; the poisoned JPEG is queued beside it.
    let (poisoned, healthy) = (state::file_id(&c, "a.jpg").unwrap(), state::file_id(&c, "b.png").unwrap());
    for id in [poisoned, healthy] {
        c.queue_rotation(id, Turns::new(1)).unwrap();
    }

    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(
        ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap(),
    );
    let writer = Writer::start_with_fault(
        catalog_path.clone(),
        thumbs,
        Arc::new(Bus::new(16)),
        Fault::PanicInProcess(poisoned),
        RecordingSink::new(),
    )
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
    let dir = pg_testkit::run_folder("scratch_library", "drainpanic").unwrap();
    std::fs::create_dir_all(dir.join("images")).unwrap();
    make_files(&dir).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    c.queue_rotation(state::file_id(&c, "b.png").unwrap(), Turns::new(1)).unwrap();

    let (events, quiet) = Quiet::bus();
    let thumbs = Arc::new(
        ThumbCache::open(
            &dir.join("thumbs.sqlite"),
            &c.catalog_id().unwrap(),
            pg_model::cache::ThumbCap::from_bytes(1 << 30),
            pg_model::versions::THUMBS_VERSION,
            events,
        )
        .unwrap(),
    );
    let writer = Writer::start_with_fault(
        catalog_path.clone(),
        thumbs,
        Arc::new(Bus::new(16)),
        Fault::PanicInDrain,
        RecordingSink::new(),
    )
    .unwrap();
    let start = Instant::now();
    while writer.is_running() {
        assert!(start.elapsed() < Duration::from_secs(60), "the writer is still running");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(c.left_to_write().unwrap(), 1, "the write is still queued");
    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}

/// A folder whose removal was begun, with nothing waiting to be written: an add over it is refused while the removal is pending,
/// and once the removal has finished the add goes through (`pg-driver` finishes the removal and adds the folder, in that order).
#[test]
fn an_add_over_a_pending_removal_is_refused_until_the_removal_finishes() {
    let (dir, _files, _catalog_path, c) = imported("addfinish").unwrap();
    let images = dir.join("images");
    let root = c.roots().unwrap().remove(0);
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    assert!(
        matches!(
            pg_library::folders::add_folder(&c, &images),
            Err(pg_library::folders::AddError::Catalog(pg_catalog::Error::RemovalPending { .. }))
        ),
        "the add is refused while the removal is pending"
    );
    let driven = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert!(driven.files > 0, "its files were deleted");
    assert!(c.removal_markers().unwrap().is_empty());
    let (added, new) = pg_library::folders::add_folder(&c, &images).unwrap();
    assert!(new, "the folder is added again");
    let roots = c.roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert!(c.rules().same(&roots[0].path, &added));
    drop(c);
    quiet.check();
}

/// A folder whose removal is kept for a change still waiting to be written (here the folder is still there, and the writer is not
/// running): the finish does what it can, the removal stays pending, and an add over it is still refused and adds nothing.
#[test]
fn a_removal_kept_for_a_write_still_refuses_an_add() {
    let (dir, _files, _catalog_path, c) = imported("addkept").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(on_disk(&c, &db_path), on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    let images = dir.join("images");
    let root = c.roots().unwrap().remove(0);
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let driven = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!(driven.deferred, 1, "the file with the change is kept");
    assert_eq!(c.removal_markers().unwrap().len(), 1, "the removal is still pending");
    assert!(
        matches!(
            pg_library::folders::add_folder(&c, &images),
            Err(pg_library::folders::AddError::Catalog(pg_catalog::Error::RemovalPending { .. }))
        ),
        "the add is still refused"
    );
    assert!(c.roots().unwrap().is_empty(), "nothing was added");
    drop(c);
    quiet.check();
}

/// A folder removed while a change to one of its files still waits for the writer, and the folder is not where it was
/// (an unplugged drive, here moved away): the removal keeps that file's row and its temp, the writer keeps the change
/// queued, and only the user's discard gives the change up (with its temp).
#[test]
fn a_removal_keeps_a_change_whose_folder_is_gone_until_it_is_discarded() {
    let (dir, _files, catalog_path, c) = imported("removalgone").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();
    let images = dir.join("images");
    let away = dir.join("images-away");
    std::fs::rename(&images, &away).unwrap();

    let root = c.roots().unwrap().remove(0);
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let driven = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!(driven.deferred, 1, "the file with a change is kept");
    assert_eq!(c.status().unwrap().missing, 1, "the rest of the folder went, and that one stayed (hidden)");

    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    // Long enough for the writer to look at the change once (its folder is not there, so it waits).
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(state::writes_queued(&c, id).unwrap(), 1, "the change is still queued");
    assert_eq!(c.removal_markers().unwrap().first().map(|m| m.deferred), Some(1));
    drop(writer);

    let discarded = c.discard_removal(&root.path).unwrap().expect("the removal is pending");
    assert_eq!((discarded.rows, discarded.temps.len()), (1, 1));
    assert!(c.removal_markers().unwrap().is_empty());
    std::fs::rename(&away, &images).unwrap();
    drop(c);
    quiet.check();
}

/// A removal's deletes end with a PASSIVE checkpoint, on the catalog that made them (section 11).
#[test]
fn a_removal_checkpoints_the_wal_after_its_deletes() {
    let (dir, _files, _catalog_path, c) = imported("removal-checkpoint").unwrap();
    let root = c.roots().unwrap().remove(0);
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let before = c.passive_checkpoints();
    let driven = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert!(driven.files > 0, "its files were deleted");
    assert!(c.passive_checkpoints() > before, "the deletes were checkpointed");
    drop(c);
    quiet.check();
}

/// A folder removed while its files have a change waiting, and the folder is still there: the writer finishes each
/// change from its temp (the whole copy it kept), and then the removal deletes the files, for good.
#[test]
fn a_removal_finishes_a_change_whose_folder_is_still_there() {
    let (dir, _files, catalog_path, c) = imported("removalrecover").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    let temp = pg_writeback::temp_of(&db_path, WriteKind::Rating);
    std::fs::copy(on_disk(&c, &db_path), on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, id, "rating", 3, &temp).unwrap();

    let root = c.roots().unwrap().remove(0);
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let kept = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!(kept.deferred, 1, "the file with a change waits for the writer");
    assert_eq!(c.status().unwrap().missing, 1, "the rest of the folder went, and that one stayed (hidden)");

    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the change is still queued",
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    assert!(!on_disk(&c, &temp).exists(), "the writer finished the copy and took its temp");
    drop(writer);

    let done = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!((done.files, done.deferred), (1, 0));
    assert!(c.removal_markers().unwrap().is_empty());
    assert_eq!(c.status().unwrap().files, 0, "the removed folder's files are gone");
    drop(c);
    quiet.check();
}

/// The gate the test below holds its copy at: a static, because the writer's fault names it.
static HELD_COPY: PauseGate = PauseGate::new();

/// Opens [`HELD_COPY`] when dropped. The test declares it after its writer, so it drops first: a failed assertion then does
/// not leave the writer's thread held at the gate, which the writer's shutdown would wait for.
struct OpenHeldCopy;

impl Drop for OpenHeldCopy {
    fn drop(&mut self) {
        HELD_COPY.open();
    }
}

/// A removal of a folder while a copy of one of its files is in progress: the writer has picked the job, and it waits
/// before it opens the original. The removal hides the file, and its change waits for the writer (the file is deferred).
/// The copy goes on and finishes, the writer takes its temp, and the removal then finishes the folder (section 1: a hide
/// that lands after the pick, which the removal rule covers).
#[test]
fn a_copy_in_progress_when_its_folder_is_removed_is_finished_and_the_removal_follows() {
    let (dir, _files, catalog_path, c) = imported("copy_during_removal").unwrap();
    let (id, db_path) = state::file_and_path(&c, "b.png").unwrap();
    c.set_rating(id, Rating::new(3), true).unwrap();
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer = Writer::start_with_fault(
        catalog_path.clone(),
        thumbs.clone(),
        Arc::new(Bus::new(16)),
        Fault::PauseCopy { file: id, gate: &HELD_COPY },
        RecordingSink::new(),
    )
    .unwrap();
    let _open = OpenHeldCopy;
    writer.wake();
    pg_testkit::wait_until(Duration::from_secs(60), || "the copy is held", || HELD_COPY.entered());

    let root = c.roots().unwrap().remove(0);
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let hidden = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!(hidden.deferred, 1, "the file whose copy is in progress waits for the writer");

    HELD_COPY.open();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the writer finishes the copy",
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    drop(writer);
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the copy in progress reached the file");
    assert!(!on_disk(&c, &pg_writeback::temp_of(&db_path, WriteKind::Rating)).exists(), "the writer took its temp");

    let done = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!((done.files, done.deferred), (1, 0));
    assert!(c.removal_markers().unwrap().is_empty());
    assert_eq!(c.status().unwrap().files, 0, "the removed folder's files are gone");
    drop(c);
    quiet.check();
}

/// A folder removed and added again: its pictures come back with their analyses, and the rating written into a file comes
/// back with the file (the manual check "remove, then re-add", section 1).
#[test]
fn a_removed_folder_added_again_gets_its_analyses_and_its_ratings_back() {
    let (dir, _files, catalog_path, mut c) = imported("readd").unwrap();
    let (id, _) = state::file_and_path(&c, "b.png").unwrap();
    let analysed = state::analysis_sizes(&c).unwrap().len();
    assert!(analysed > 0, "the folder's pictures are analysed before the removal");
    c.set_rating(id, Rating::new(3), true).unwrap();
    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the rating is written",
        || state::writes_queued(&c, id).unwrap() == 0 && writer.status().error.is_none(),
    );
    drop(writer);
    assert_eq!(state::file_rating_of(&c, id).unwrap(), Some(3), "the rating is in the file");

    let root = c.roots().unwrap().remove(0);
    assert!(matches!(c.begin_removal(root.id, &root.path).unwrap(), pg_model::removal::Begun::Started { .. }));
    let gone = pg_library::removals::finish(&c, &thumbs, &mut |_| true).unwrap();
    assert_eq!(gone.deferred, 0, "nothing was waiting to be written");
    assert_eq!(c.status().unwrap().files, 0, "the folder's files are gone");

    // Added again: scanned and imported, as the app does a folder it is given.
    assert!(c.add_root(&root.path).unwrap());
    pg_scan::scan(&mut c, &root.path, &Control::new()).unwrap();
    pg_import::import(
        &mut c,
        &Options::new(pg_platform::priority::Priority::Normal),
        &pg_catalog::VolumeSet::All,
        &Control::new(),
    )
    .unwrap();
    let (again, _) = state::file_and_path(&c, "b.png").unwrap();
    assert_eq!(state::analysis_sizes(&c).unwrap().len(), analysed, "the analyses are back");
    assert_eq!(state::rating_of(&c, again).unwrap(), Some(3), "the rating comes back with the file");
    drop(c);
    quiet.check();
}

/// A copy waiting on a file that is gone (missing) comes first in the queue, and a rating on another file is queued behind
/// it: the rating is written while the copy waits, and the copy keeps its temp (the only whole copy) until the file is back.
#[test]
fn a_waiting_copy_does_not_hold_back_the_other_writes() {
    let (dir, files, catalog_path, c) = imported("waitingcopy").unwrap();
    let (b, b_path) = state::file_and_path(&c, "b.png").unwrap();
    let (a, _) = state::file_and_path(&c, "a.jpg").unwrap();
    let temp = pg_writeback::temp_of(&b_path, WriteKind::Rating);
    std::fs::copy(&files[1].0, on_disk(&c, &temp)).unwrap();
    state::queue_finished_copy(&c, b, "rating", 3, &temp).unwrap();
    state::queue_write(&c, a, "rating", 4).unwrap();
    state::mark_missing(&c, b).unwrap();

    let (thumbs, quiet) = thumbs(&dir, &c).unwrap();
    let writer =
        Writer::start(catalog_path.clone(), thumbs.clone(), Arc::new(Bus::new(16)), RecordingSink::new()).unwrap();
    writer.wake();
    pg_testkit::wait_until(
        Duration::from_secs(60),
        || "the other rating is still queued behind the waiting copy",
        || state::writes_queued(&c, a).unwrap() == 0,
    );
    assert_eq!(state::file_rating_of(&c, a).unwrap(), Some(4));
    assert_eq!(state::writes_queued(&c, b).unwrap(), 1, "the copy waits");
    assert!(on_disk(&c, &temp).exists(), "its temp is kept");

    drop(writer);
    drop(c);
    let _ = std::fs::remove_dir_all(dir.join("images"));
    quiet.check();
}
