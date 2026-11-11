//! The borders pass on real files: a file that can't be read stays due for the next pass, so it is looked at again once
//! it can be read. Another program holds the file (pg_testkit::held): on Windows it is open without sharing, on Unix it
//! has no read permission, and the hold lets go before the next pass. A file that doesn't decode is recorded in the
//! failure ledger, and leaves it once its borders are found, in the transaction that stores the borders: a batch that
//! can't be written stores neither.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::notice::FilePass;
use pg_testkit::held::{Access, hold};

/// Held by each test that runs the pass: the detector's fault is the process's, and file ids repeat from catalog to
/// catalog.
static ONE_PASS: Mutex<()> = Mutex::new(());

/// A run folder with `images/a.png` (96x80, a gradient), scanned and imported into a catalog beside it: the image's path,
/// its bytes, and the catalog.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn imported(prefix: &str) -> (std::path::PathBuf, Vec<u8>, Catalog) {
    let dir = pg_testkit::run_folder("scratch_borders", prefix).unwrap();
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let byte = |v: u32| u8::try_from(v % 256).unwrap();
    let a = images.join("a.png");
    image::RgbImage::from_fn(96, 80, |x, y| image::Rgb([byte(x * 4), byte(y * 5), 128])).save(&a).unwrap();
    let good = std::fs::read(&a).unwrap();
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &opts(), false, &ctl).unwrap();
    (a, good, c)
}

fn opts() -> Options {
    Options::new(pg_platform::priority::Priority::Normal)
}

/// `a` rewritten in place with `bytes`, keeping its time: at the same size, the catalog has the file as it was.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn rewrite(a: &Path, bytes: &[u8]) {
    let mtime = std::fs::metadata(a).unwrap().modified().unwrap();
    std::fs::write(a, bytes).unwrap();
    std::fs::File::options().write(true).open(a).unwrap().set_modified(mtime).unwrap();
}

/// `a` damaged in place after the import, at the same size and time (as a disk error would leave it): the detector
/// can't decode it.
fn damage(a: &Path, len: usize) {
    rewrite(a, &vec![0; len]);
}

#[test]
fn an_unreadable_file_stays_due_for_the_next_pass() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = pg_testkit::run_folder("scratch_borders", "borders").unwrap();
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    // A channel wraps past 255.
    let byte = |v: u32| u8::try_from(v % 256).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbImage::from_fn(96, 80, |x, y| image::Rgb([byte(x * 4), byte(y * 5), 128]))
            .save(images.join(name))
            .unwrap();
    }
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    assert_eq!((state::borders_due(&c, "a.png").unwrap(), state::borders_due(&c, "b.png").unwrap()), (1, 1));

    // a.png is held: the pass can't read it. Where the permissions don't stop this process (root), there is nothing to
    // test.
    let Some(lock) = hold(&images.join("a.png"), Access::Read).unwrap() else {
        pg_testkit::skip("the permissions don't stop this process from reading (root)");
        return;
    };
    let stats = pg_borders::find(&mut c, &opts, &ctl).unwrap();
    // Both looked at; the unreadable one stored nothing, and is no failure of the file's.
    assert_eq!((stats.files, stats.failed, stats.unreadable, stats.stored), (2, 1, 1, 1));
    assert!(c.recorded_failures(FilePass::Borders).unwrap().is_empty());
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 1, "an unreadable file must stay due");
    assert_eq!(state::borders_due(&c, "b.png").unwrap(), 0, "the readable one is done");

    // Readable again: the next pass looks at it, and it is no longer due.
    drop(lock);
    let stats = pg_borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed, stats.stored), (1, 0, 1));
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 0);

    drop(c);
    let _ = std::fs::remove_dir_all(Path::new(&images));
}

/// A file that doesn't decode is stored as having none, so the pass doesn't look at it again until it changes, and the
/// ledger has it; once its borders are found (here, looked for again as an analysis made again asks) it leaves it.
#[test]
fn a_file_that_doesnt_decode_is_recorded_until_its_borders_are_found() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = pg_testkit::run_folder("scratch_borders", "borders-ledger").unwrap();
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let byte = |v: u32| u8::try_from(v % 256).unwrap();
    let a = images.join("a.png");
    image::RgbImage::from_fn(96, 80, |x, y| image::Rgb([byte(x * 4), byte(y * 5), 128])).save(&a).unwrap();
    let good = std::fs::read(&a).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    pg_import::import(&mut c, &opts, false, &ctl).unwrap();
    // Damaged in place after the import, at the same size and time: the catalog has the file as it was.
    let mtime = std::fs::metadata(&a).unwrap().modified().unwrap();
    let rewrite = |bytes: &[u8]| {
        std::fs::write(&a, bytes).unwrap();
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(mtime).unwrap();
    };
    rewrite(&vec![0; good.len()]);
    let stats = pg_borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed, stats.unreadable, stats.stored), (1, 1, 0, 1));
    let [failed] = &c.failed_files(FilePass::Borders, 10).unwrap()[..] else { panic!("one failure") };
    assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "a.png").unwrap(), false));
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 0, "not looked at again");
    // Mended, and due again: found, and gone from the ledger.
    rewrite(&good);
    state::make_borders_due(&c, "a.png").unwrap();
    let stats = pg_borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed), (1, 0));
    assert!(c.recorded_failures(FilePass::Borders).unwrap().is_empty());
    drop(c);
    let _ = std::fs::remove_dir_all(Path::new(&dir));
}

/// A detector that panics on a file records the file, as one that doesn't decode does: it isn't looked at again until
/// it changes, and the pass goes on.
#[test]
fn a_detector_panic_records_the_file() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (_a, _good, mut c) = imported("borders-panic");
    let id = state::file_id(&c, "a.png").unwrap();
    pg_borders::fault::panic_on(Some(id));
    let stats = pg_borders::find(&mut c, &opts(), &Control::new());
    pg_borders::fault::panic_on(None);
    let stats = stats.unwrap();
    assert_eq!((stats.files, stats.failed, stats.unreadable, stats.stored), (1, 1, 0, 1));
    let [failed] = &c.failed_files(FilePass::Borders, 10).unwrap()[..] else { panic!("one failure") };
    assert_eq!((failed.file_id, failed.retry), (id, false));
    assert!(failed.error.contains("a fault"), "{}", failed.error);
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 0, "not looked at again");
}

/// A file that fails again keeps its row as it was (its serial, so it is not news again): the failure path never
/// clears its own row.
#[test]
fn a_file_that_fails_again_keeps_its_row() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (a, good, mut c) = imported("borders-again");
    damage(&a, good.len());
    pg_borders::find(&mut c, &opts(), &Control::new()).unwrap();
    let entries = c.ledger_entries(false).unwrap();
    assert_eq!(entries.len(), 1, "the borders' entry");
    state::make_borders_due(&c, "a.png").unwrap();
    let stats = pg_borders::find(&mut c, &opts(), &Control::new()).unwrap();
    assert_eq!((stats.files, stats.failed), (1, 1));
    assert_eq!(c.recorded_failures(FilePass::Borders).unwrap().len(), 1);
    assert_eq!(c.ledger_entries(false).unwrap(), entries, "the row and its serial are as they were");
}

/// A batch that can't be written (here, its ledger write fails) stores nothing, neither the borders nor the ledger's
/// rows: its files stay due, and the pass fails.
#[test]
fn a_batch_that_cant_be_written_stores_nothing_and_leaves_its_files_due() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (a, good, mut c) = imported("borders-locked");
    damage(&a, good.len());
    let failing = pg_catalog::fixture::LedgerWritesFail::start();
    let found = pg_borders::find(&mut c, &opts(), &Control::new());
    drop(failing);
    assert!(found.is_err(), "the batch couldn't be written");
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 1, "the file stays due");
    assert!(c.recorded_failures(FilePass::Borders).unwrap().is_empty(), "nor is its failure recorded");
}

/// The ledger's changes the pass makes are announced (`Control::ledger_changed`): a recorded failure, and a row that
/// goes when its borders are found.
#[test]
fn the_ledgers_changes_are_announced() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (a, good, mut c) = imported("borders-announced");
    let ctl = Control::new();
    let announced = Arc::new(AtomicUsize::new(0));
    let counted = announced.clone();
    ctl.on_ledger_change(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    damage(&a, good.len());
    pg_borders::find(&mut c, &opts(), &ctl).unwrap();
    assert_eq!(announced.load(Ordering::SeqCst), 1);
    rewrite(&a, &good);
    state::make_borders_due(&c, "a.png").unwrap();
    pg_borders::find(&mut c, &opts(), &ctl).unwrap();
    assert_eq!(announced.load(Ordering::SeqCst), 2);
    assert!(c.recorded_failures(FilePass::Borders).unwrap().is_empty());
}
