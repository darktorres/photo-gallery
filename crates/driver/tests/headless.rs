//! The driver over the app's library, on scratch folders: a run imports what it is asked to, leaves what a chosen pass set does not
//! run owed for a later run, refuses to write without a writer, and finishes a removal.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pg_catalog::Catalog;
use pg_driver::{Error, Headless, Setup};
use pg_library::layout::Layout;
use pg_library::mask::{PassKind, PassMask};
use pg_model::turns::Turns;
use pg_platform::native::PATH_RULES;

/// How long a run may take in a test: generous, so a loaded machine does not fail one.
const WAIT: Duration = Duration::from_secs(120);

/// A scratch folder with a `home` and an `images` folder, and small PNGs named `names` in `images`. Returns the scratch folder,
/// the home and the images.
fn library(prefix: &str, names: &[&str]) -> Result<(PathBuf, PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let dir = pg_testkit::run_folder_with("pg_driver_tests", prefix, &["home", "images"])?;
    let pixels: Vec<u8> =
        (0..8u32 * 8 * 3).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap_or(0)).collect();
    let images = dir.join("images");
    for name in names {
        std::fs::write(images.join(name), pg_codec::fixtures::png_rgb(8, 8, &pixels)?)?;
    }
    Ok((dir.clone(), dir.join("home"), images))
}

/// The tests' setup: the home is made when it is missing (the library helper makes the folder; the catalog is made on open).
fn setup(home: &Path) -> Setup {
    Setup { create: true, ..Setup::new(home) }
}

#[test]
fn a_run_imports_the_folder_and_finishes_quiet() {
    let (dir, home, images) = library("dev-run", &["a.png", "b.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let status = headless.status().unwrap();
    assert_eq!(status.counts.files, 2, "{status:?}");
    assert_eq!(status.counts.analyzed, 2, "{status:?}");
    assert_eq!(status.counts.pending, 0, "{status:?}");
    assert_eq!(status.roots.len(), 1, "{status:?}");
    assert_eq!(status.removals, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pass_left_out_owes_its_work_until_a_run_runs_it() {
    let (dir, home, images) = library("dev-owed", &["a.png", "b.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();

    // Only the scan runs: the files are found, and their analyses are owed.
    headless.run(PassMask::only(&[PassKind::Scans]), WAIT).unwrap();
    let scanned = headless.status().unwrap();
    assert_eq!(scanned.counts.files, 2, "{scanned:?}");
    assert_eq!(scanned.counts.analyzed, 0, "{scanned:?}");
    assert_eq!(scanned.counts.pending, 2, "{scanned:?}");

    // The next run runs the import, and nothing is lost.
    headless.run(PassMask::ALL, WAIT).unwrap();
    let imported = headless.status().unwrap();
    assert_eq!(imported.counts.analyzed, 2, "{imported:?}");
    assert_eq!(imported.counts.pending, 0, "{imported:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_without_a_writer_refuses_to_write() {
    let (dir, home, _images) = library("dev-no-writer", &[]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    assert!(matches!(headless.write_pending(WAIT), Err(Error::WriterOff)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_removal_finishes_before_the_run_returns() {
    let (dir, home, images) = library("dev-remove", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let path = images.to_string_lossy().into_owned();
    assert!(headless.remove_folder(&path, WAIT).unwrap(), "the folder is in the library");
    let status = headless.status().unwrap();
    assert!(status.roots.is_empty(), "{status:?}");
    assert_eq!(status.removals, 0, "{status:?}");
    assert_eq!(status.counts.files, 0, "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_queued_change_keeps_the_job_busy_while_no_writer_runs() {
    let (dir, home, images) = library("dev-queued", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();
    assert!(headless.quiescent(), "the imported library is quiet");

    // A turn queued in the catalog is written by the writer, which this run does not have: the job owes that write.
    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let folder = PATH_RULES.parse(&images.to_string_lossy()).unwrap();
    let file = catalog.files_under(&folder).unwrap()[0];
    catalog.queue_rotation(file, Turns::new(1)).unwrap();
    assert!(!headless.quiescent(), "a queued write is work the job owes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_removal_left_unfinished_is_finished_when_the_job_next_starts() {
    let (dir, home, images) = library("dev-unfinished", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    // A removal begun in the catalog, as a run that stopped half way would leave it: the job owes its finish.
    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let root = catalog.roots().unwrap().remove(0);
    catalog.begin_removal(root.id, &root.path).unwrap();
    drop(catalog);
    drop(headless);

    // The next start of the job finishes it, before it is quiet.
    let next = Headless::open(&setup(&home)).unwrap();
    next.wait_quiet(WAIT).unwrap();
    let status = next.status().unwrap();
    assert!(status.roots.is_empty(), "{status:?}");
    assert_eq!(status.removals, 0, "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_the_scan_cannot_walk_is_kept_by_the_ledger_and_the_others_still_scan() {
    // The folders a, b and c each hold a picture; b is removed after the first run. The second run refuses it (the folder ledger
    // keeps it) and scans a and c with their new pictures.
    let dir = pg_testkit::run_folder_with("pg_driver_tests", "dev-refused", &["home", "a", "b", "c"]).unwrap();
    let picture = || pg_codec::fixtures::png_rgb(4, 4, &[0; 48]).unwrap();
    for folder in ["a", "b", "c"] {
        std::fs::write(dir.join(folder).join("1.png"), picture()).unwrap();
    }
    let home = dir.join("home");
    let headless = Headless::open(&setup(&home)).unwrap();
    for folder in ["a", "b", "c"] {
        headless.add_folder(&dir.join(folder)).unwrap();
    }
    headless.run(PassMask::ALL, WAIT).unwrap();

    // The new pictures are made before b is removed: on Linux a file made after a removal can take the removed file's inode.
    for folder in ["a", "c"] {
        std::fs::write(dir.join(folder).join("2.png"), picture()).unwrap();
    }
    std::fs::remove_dir_all(dir.join("b")).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let status = headless.status().unwrap();
    assert_eq!(status.counts.files, 5, "{status:?}");
    assert_eq!(status.folder_failures, 1, "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_added_over_an_unfinished_removal_finishes_the_removal_first() {
    let (dir, home, images) = library("dev-readd", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    // A removal begun in the catalog and left unfinished, as a stopped app leaves one.
    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let root = catalog.roots().unwrap().remove(0);
    catalog.begin_removal(root.id, &root.path).unwrap();
    drop(catalog);
    drop(headless);

    // Adding the folder again finishes that removal first, then adds the folder and scans it.
    let again = Headless::open(&setup(&home)).unwrap();
    again.add_folder(&images).unwrap();
    again.run(PassMask::ALL, WAIT).unwrap();
    let status = again.status().unwrap();
    assert_eq!(status.removals, 0, "{status:?}");
    assert_eq!(status.roots.len(), 1, "{status:?}");
    assert_eq!(status.counts.files, 1, "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_roots_on_one_volume_are_scanned_and_imported_together() {
    // Two folders on one volume are one lane; the test holds that both are scanned, imported and counted.
    let dir = pg_testkit::run_folder_with("pg_driver_tests", "dev-two-roots", &["home", "a", "b"]).unwrap();
    for folder in ["a", "b"] {
        for name in ["1.png", "2.png"] {
            std::fs::write(dir.join(folder).join(name), pg_codec::fixtures::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        }
    }
    let headless = Headless::open(&setup(&dir.join("home"))).unwrap();
    for folder in ["a", "b"] {
        headless.add_folder(&dir.join(folder)).unwrap();
    }
    headless.run(PassMask::ALL, WAIT).unwrap();
    let status = headless.status().unwrap();
    assert_eq!(status.counts.files, 4, "{status:?}");
    assert_eq!(status.counts.analyzed, 4, "{status:?}");
    assert_eq!(status.roots.len(), 2, "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_removed_folder_takes_its_kept_notices_and_the_others_stay() {
    use pg_model::notice::{Notice, ScanNote};
    use pg_model::rules::Path as RulePath;

    let dir = pg_testkit::run_folder_with("pg_driver_tests", "dev-notices", &["home", "a", "b"]).unwrap();
    for folder in ["a", "b"] {
        std::fs::write(dir.join(folder).join("1.png"), pg_codec::fixtures::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
    }
    let headless = Headless::open(&setup(&dir.join("home"))).unwrap();
    for folder in ["a", "b"] {
        headless.add_folder(&dir.join(folder)).unwrap();
    }
    headless.run(PassMask::ALL, WAIT).unwrap();

    let root = |folder: &str| pg_platform::names::normalize_path(&dir.join(folder)).unwrap();
    let note = |root: &RulePath| Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 1 };
    let catalog = Catalog::open(&Layout::in_home(dir.join("home")).catalog_path(), PATH_RULES).unwrap();
    catalog.post_notice(&note(&root("a")), 0).unwrap();
    catalog.post_notice(&note(&root("b")), 0).unwrap();
    assert_eq!(catalog.inbox_entries(10, false).unwrap().len(), 2);
    drop(catalog);

    assert!(headless.remove_folder(&dir.join("a").to_string_lossy(), WAIT).unwrap());
    let catalog = Catalog::open(&Layout::in_home(dir.join("home")).catalog_path(), PATH_RULES).unwrap();
    let kept: Vec<Notice> = catalog.inbox_entries(10, false).unwrap().into_iter().map(|entry| entry.notice).collect();
    assert_eq!(kept, [note(&root("b"))]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_of_one_pass_with_the_scan_left_out_still_finishes() {
    // The folder's walk is asked for by `run` (`rescan`), and the scan pass is left out, so the walk waits for a run that has it:
    // that wait is not work this run owes, and the run finishes.
    let (dir, home, images) = library("dev-one-pass", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();
    headless.run(PassMask::only(&[PassKind::Thumbs]), WAIT).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_whose_walk_another_process_holds_keeps_the_job_busy_until_it_is_released() {
    let (dir, home, images) = library("dev-busy", &["a.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    // Another walk holds the folder's lease: a rescan is refused as busy, and retried, so the job is not quiet while it holds it.
    let mut catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let root = catalog.roots().unwrap().remove(0).path;
    let lease = catalog.begin_scan(&root).unwrap();
    let held = headless.run(PassMask::ALL, Duration::from_secs(3));
    assert!(matches!(held, Err(Error::Timeout { .. })), "a busy folder keeps the job busy: {held:?}");
    drop(lease);
    headless.wait_quiet(WAIT).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_write_the_files_refuse_ends_the_write_with_the_writers_words() {
    // A read-only original cannot take its rotation: the writer's job fails, and the write ends with that error, not at the timeout.
    let (dir, home, images) = library("dev-write-refused", &["a.png"]).unwrap();
    let mut writing = setup(&home);
    writing.writer = true;
    let headless = Headless::open(&writing).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let original = images.join("a.png");
    set_readonly(&original, true).unwrap();
    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let folder = PATH_RULES.parse(&images.to_string_lossy()).unwrap();
    let file = catalog.files_under(&folder).unwrap()[0];
    catalog.queue_rotation(file, Turns::new(1)).unwrap();

    let result = headless.write_pending(WAIT);
    set_readonly(&original, false).unwrap();
    assert!(matches!(result, Err(Error::WriteFailed { .. })), "{result:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_that_was_not_interrupted_finishes_every_scan_it_began() {
    // The scans a run begins are each finished by their walk. A walk cut short leaves its row unfinished; an uninterrupted run leaves none.
    let (dir, home, images) = library("dev-scans-finished", &["a.png", "b.png"]).unwrap();
    let headless = Headless::open(&setup(&home)).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();
    std::fs::write(images.join("c.png"), pg_codec::fixtures::png_rgb(8, 8, &[0; 192]).unwrap()).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    assert_eq!(catalog.unfinished_scans().unwrap(), 0, "every scan the runs began is finished");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Makes the file read-only, or writable again.
fn set_readonly(path: &Path, readonly: bool) -> std::io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(readonly);
    std::fs::set_permissions(path, permissions)
}
