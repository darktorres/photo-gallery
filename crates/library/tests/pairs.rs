//! The pairing pass on real files: the import leaves the pictures' pairs owed (`pairs_owed`), the pass makes them,
//! a stop leaves what is owed owed, and a write that fails stores none of its batch, so the next pass makes them all.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_platform::priority::Priority;

/// Held by each test: the fault is a trigger in a catalog, and the file ids of two catalogs are the same numbers.
static ONE_PASS: Mutex<()> = Mutex::new(());

/// A run folder with `images/a.png` and `images/b.png` (the same textured picture) and `images/c.png` (another one),
/// scanned and imported into a catalog beside them: the catalog, the run's control, and the folder.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn imported(prefix: &str) -> (Catalog, Control, PathBuf) {
    let dir = pg_testkit::run_folder("scratch_pairs", prefix).unwrap();
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let byte = |v: u32| u8::try_from(v % 256).unwrap();
    let same = image::RgbImage::from_fn(96, 80, |x, y| {
        image::Rgb([byte(x * x + y * 31), byte(x * 7 + y * y), byte((x ^ y) * 5)])
    });
    same.save(images.join("a.png")).unwrap();
    same.save(images.join("b.png")).unwrap();
    image::RgbImage::from_fn(96, 80, |x, y| image::Rgb([byte(y * y + 3 * x), byte(x * 13 + 5), byte(y * 3)]))
        .save(images.join("c.png"))
        .unwrap();
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &opts(), false, &ctl).unwrap();
    (c, ctl, dir)
}

fn opts() -> Options {
    Options::new(Priority::Normal)
}

/// The pass's pairs of `a.png` and `b.png` (the same picture), in the order of their ids, as the pass stores them.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn the_twins(c: &Catalog) -> (i64, i64) {
    let (a, b) = (state::file_id(c, "a.png").unwrap().get(), state::file_id(c, "b.png").unwrap().get());
    (a.min(b), a.max(b))
}

#[test]
fn the_import_owes_the_pairs_and_the_pass_makes_them() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("pass");
    // Nothing is paired by the import; the pictures owe their pairs.
    assert!(state::pairs(&c).unwrap().is_empty());
    assert!(state::owes_pairs(&c, "a.png").unwrap());
    assert!(state::owes_pairs(&c, "b.png").unwrap());
    assert!(c.owed_pairs().unwrap() >= 2);

    let stats = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(stats.complete);
    let (lo, hi) = the_twins(&c);
    assert!(state::pairs(&c).unwrap().contains(&(lo, hi, 0)), "the same picture pairs at distance 0");
    assert!(!state::owes_pairs(&c, "a.png").unwrap());
    assert!(!state::owes_pairs(&c, "b.png").unwrap());
    assert_eq!(c.owed_pairs().unwrap(), 0);

    // Nothing owed: the next pass is one probe, and makes nothing.
    let again = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(again.complete);
    assert_eq!(again.files, 0);
}

#[test]
fn a_stop_before_the_pass_writes_nothing_and_the_pairs_stay_owed() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("stop");
    let owed = c.owed_pairs().unwrap();
    ctl.request_stop();
    let stopped = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(!stopped.complete, "a stop is not a complete pass");
    assert!(state::pairs(&c).unwrap().is_empty());
    assert_eq!(c.owed_pairs().unwrap(), owed);
    // The next pass makes them all.
    ctl.clear_stop();
    let resumed = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(resumed.complete);
    let (lo, hi) = the_twins(&c);
    assert!(state::pairs(&c).unwrap().contains(&(lo, hi, 0)));
    assert_eq!(c.owed_pairs().unwrap(), 0);
}

/// The pass on a slice of one owed picture (a pass of several slices on this small library) makes the pairs one slice
/// makes, and a catalog's page cache is the app's again after the pass, which bounded it while it ran.
#[test]
fn slices_of_one_make_the_pairs_one_slice_makes() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("slices");
    let whole = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    let pairs = state::pairs(&c).unwrap();
    c.pairs_again().unwrap();
    let settings = pg_import::pairs::Settings { slice: 1, ..pg_import::pairs::Settings::default() };
    let sliced = pg_import::pairs::pair_with(&mut c, &opts(), settings, &ctl).unwrap();
    assert!(sliced.complete);
    assert_eq!((sliced.files, sliced.pairs), (whole.files, whole.pairs));
    assert_eq!(state::pairs(&c).unwrap(), pairs);
    assert_eq!(c.owed_pairs().unwrap(), 0);
    assert_eq!(state::cache_kib(&c).unwrap(), 262_144, "the app's cache, after the pass");
}

#[test]
fn a_batch_that_cannot_be_written_stores_nothing_and_the_next_pass_makes_it_all() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("fault");
    let owed = c.owed_pairs().unwrap();
    state::fail_pair_writes(&c).unwrap();
    assert!(pg_import::pairs::pair(&mut c, &opts(), &ctl).is_err(), "the write's failure is the pass's");
    // The batch's transaction rolled back: no pair, and every picture still owes its pairs.
    assert!(state::pairs(&c).unwrap().is_empty());
    assert!(state::owes_pairs(&c, "a.png").unwrap());
    assert_eq!(c.owed_pairs().unwrap(), owed);
    state::allow_pair_writes(&c).unwrap();
    let made = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(made.complete);
    let (lo, hi) = the_twins(&c);
    assert!(state::pairs(&c).unwrap().contains(&(lo, hi, 0)));
    assert_eq!(c.owed_pairs().unwrap(), 0);
}
