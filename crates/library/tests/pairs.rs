//! The pairing pass on real files: the import leaves the pictures' pairs owed (`pairs_owed`), the pass makes them,
//! a stop leaves what is owed owed, and a write that fails stores none of its batch, so the next pass makes them all.
//! A pair is made with a partner whose file is away (missing or pending), so it is there when the file is back
//! without a new analysis, however it comes back.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_catalog::queue::WriteEffect;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::turns::Turns;
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
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    pg_import::import(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    // The Hashes pass makes the hashes the import leaves owed: the pairs are made from them.
    pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    (c, ctl, dir)
}

fn opts() -> Options {
    Options::new(Priority::Normal)
}

/// A scan of the run folder's `images`, as [`imported`] made it.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn scan(c: &mut Catalog, dir: &Path, ctl: &Control) {
    let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(c, &root, ctl).unwrap();
}

/// The catalog of the run folder, opened again by a build of import version `import` (the others this build's).
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn reopened(c: Catalog, dir: &Path, import: i64) -> Catalog {
    drop(c);
    let versions = pg_catalog::Versions { import, ..pg_library::versions() };
    Catalog::prepare(&dir.join("catalog.sqlite"), versions, pg_platform::native::PATH_RULES).unwrap()
}

/// `a.png` analysed again (made due, then imported): it owes its pairs, and the ones it had are gone.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn analysed_again(c: &mut Catalog, ctl: &Control) {
    let a = state::file_id(c, "a.png").unwrap();
    assert_eq!(c.mark_pending(&[a]).unwrap(), 1);
    pg_import::import(c, &opts(), false, &pg_catalog::VolumeSet::All, ctl).unwrap();
    pg_import::hashes::hash(c, &opts(), &pg_catalog::VolumeSet::All, &|| true, ctl).unwrap();
}

/// Whether the import would analyse the file called `name` now (`files.pending`, of a present file).
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn due(c: &Catalog, name: &str) -> bool {
    let id = state::file_id(c, name).unwrap();
    c.pending_import(false, &pg_catalog::VolumeSet::All).unwrap().items.iter().any(|i| i.id == id)
}

/// Whether the file called `name` is present (not found missing).
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn present(c: &Catalog, name: &str) -> bool {
    state::present_files(c).unwrap().iter().any(|(_, n)| n == name)
}

/// `b.png` turned `turns` quarter turns, as the writer leaves a turn the file can't hold: kept in the catalog.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn turned(c: &Catalog, turns: u8) {
    c.queue_rotation(state::file_id(c, "b.png").unwrap(), Turns::new(turns)).unwrap();
    let job = c.next_write(0).unwrap().unwrap();
    c.write_done(&job, WriteEffect::KeptInCatalog(Turns::new(turns))).unwrap();
}

/// `b.png`'s file time, set to `mtime`.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn touched(dir: &Path, mtime: std::time::SystemTime) {
    std::fs::File::options().write(true).open(dir.join("images").join("b.png")).unwrap().set_modified(mtime).unwrap();
}

/// What a test does to the catalog of a run folder and the files in it, under the run's control.
type Step<'a> = &'a dyn Fn(Catalog, &Path, &Control) -> Catalog;

/// `a.png` and `b.png` (the same picture) paired; then `away`: `a.png` analysed again (it owes its pairs, and their
/// pair is gone) and `b.png` out of the pass (its file missing or pending) with its analysis kept, and not owing
/// anything; then the pass; then `back`: `b.png` back as it was analysed. A pair is a fact about two stored analyses,
/// so the pass made it with `b.png` away: shown again once `b.png` is back, with nothing owed and nothing to analyse.
/// Had the pass left `b.png` out, nothing would ever owe the pair again.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn the_pair_is_made_while_its_partner_is_away(prefix: &str, away: Step, back: Step) {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, dir) = imported(prefix);
    pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    let (lo, hi) = the_twins(&c);
    let (a, b) = (state::file_id(&c, "a.png").unwrap(), state::file_id(&c, "b.png").unwrap());
    assert!(state::shown_pairs(&c).unwrap().contains(&(lo, hi, 0)));

    let mut c = away(c, &dir, &ctl);
    assert!(!state::pairs(&c).unwrap().contains(&(lo, hi, 0)), "a.png's new analysis dropped its pairs");
    let work = c.pairing_work().unwrap();
    let side = |id| work.ids.iter().position(|&i| i == id).and_then(|i| work.owed.get(i).copied());
    assert_eq!(side(a), Some(true), "a.png owes its pairs");
    assert_eq!(side(b), Some(false), "b.png is a partner, and owes nothing");
    let stats = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert!(stats.complete);
    assert!(state::pairs(&c).unwrap().contains(&(lo, hi, 0)), "made with b.png away");

    let mut c = back(c, &dir, &ctl);
    assert!(state::shown_pairs(&c).unwrap().contains(&(lo, hi, 0)), "shown, with b.png back");
    assert!(
        c.pending_import(false, &pg_catalog::VolumeSet::All).unwrap().is_empty(),
        "b.png is back as it was analysed"
    );
    assert_eq!(c.owed_pairs().unwrap(), 0);
    let again = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert_eq!(again.files, 0, "nothing for a pass to make");
}

/// A volume no scan of the tests sees: the old volume of a file whose volume came back renumbered.
const LOST_VOLUME: i64 = 0x0105_7000;

#[test]
fn a_pair_made_while_its_file_was_missing_is_there_when_a_rescan_finds_it() {
    let outside = |dir: &Path| dir.join("b.png");
    the_pair_is_made_while_its_partner_is_away(
        "away-missing",
        &|mut c, dir, ctl| {
            analysed_again(&mut c, ctl);
            std::fs::rename(dir.join("images").join("b.png"), outside(dir)).unwrap();
            scan(&mut c, dir, ctl);
            assert!(!present(&c, "b.png"));
            c
        },
        // The same file under the same identity: the scan's upsert finds the row and marks it present.
        &|mut c, dir, ctl| {
            std::fs::rename(outside(dir), dir.join("images").join("b.png")).unwrap();
            scan(&mut c, dir, ctl);
            assert!(present(&c, "b.png"));
            c
        },
    );
}

#[test]
fn a_pair_made_while_its_volume_was_away_is_there_when_a_rebind_finds_it_by_name() {
    the_pair_is_made_while_its_partner_is_away(
        "away-rebind-name",
        &|mut c, _, ctl| {
            analysed_again(&mut c, ctl);
            state::on_lost_volume(&c, state::file_id(&c, "b.png").unwrap(), LOST_VOLUME, "b.png").unwrap();
            c
        },
        // The file under a new identity, in the same folder under the same name: tier 1 rebinds its row.
        &|mut c, dir, ctl| {
            scan(&mut c, dir, ctl);
            assert!(present(&c, "b.png"));
            c
        },
    );
}

#[test]
fn a_pair_made_while_its_volume_was_away_is_there_when_a_rebind_finds_it_by_file_id() {
    the_pair_is_made_while_its_partner_is_away(
        "away-rebind-id",
        &|mut c, _, ctl| {
            analysed_again(&mut c, ctl);
            state::on_lost_volume(&c, state::file_id(&c, "b.png").unwrap(), LOST_VOLUME, "lost.png").unwrap();
            c
        },
        // The file under a new identity and another name than its row's: tier 2 rebinds the row by its file ID, on a
        // volume the scan did not see.
        &|mut c, dir, ctl| {
            scan(&mut c, dir, ctl);
            assert!(present(&c, "b.png"));
            c
        },
    );
}

#[test]
fn a_pair_made_while_its_file_was_turned_is_there_when_the_turn_is_undone() {
    the_pair_is_made_while_its_partner_is_away(
        "away-turned",
        &|mut c, _, ctl| {
            analysed_again(&mut c, ctl);
            turned(&c, 1);
            assert!(due(&c, "b.png"), "a turn makes its analysis stale");
            c
        },
        &|c, _, _| {
            turned(&c, 3);
            c
        },
    );
}

#[test]
fn a_pair_made_while_its_file_had_another_time_is_there_when_the_time_is_put_back() {
    let mtime = |dir: &Path| std::fs::metadata(dir.join("images").join("b.png")).unwrap().modified().unwrap();
    // The time the file had when it was analysed, kept across the steps.
    let before = std::sync::OnceLock::new();
    the_pair_is_made_while_its_partner_is_away(
        "away-touched",
        &|mut c, dir, ctl| {
            analysed_again(&mut c, ctl);
            let was = *before.get_or_init(|| mtime(dir));
            touched(dir, was + Duration::from_secs(3600));
            scan(&mut c, dir, ctl);
            assert!(due(&c, "b.png"), "another time makes its analysis stale");
            c
        },
        &|mut c, dir, ctl| {
            touched(dir, *before.get().unwrap());
            scan(&mut c, dir, ctl);
            c
        },
    );
}

#[test]
fn a_pair_made_while_another_build_found_its_analysis_stale_is_there_when_the_first_opens_the_catalog() {
    let ours = pg_library::versions().import;
    // An older build's version: the registry starts at 1, so 0 is never a build's own and stands for any older one.
    let older = 0;
    the_pair_is_made_while_its_partner_is_away(
        "away-import-version",
        // b.png analysed by an older build, which opens the catalog last; then this build opens it again, for which
        // that analysis is stale.
        &|mut c, dir, ctl| {
            analysed_again(&mut c, ctl);
            state::set_analysis_version(&c, state::file_id(&c, "b.png").unwrap(), older).unwrap();
            let c = reopened(reopened(c, dir, older), dir, ours);
            assert!(due(&c, "b.png") && !due(&c, "a.png"));
            c
        },
        // The older build opens it again: b.png's analysis is current for it.
        &|c, dir, _| reopened(c, dir, older),
    );
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
/// makes, and a catalog's page cache is its own again after the pass, which bounded it while it ran.
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
    assert_eq!(state::cache_kib(&c).unwrap(), 8_192, "the catalog's own cache, after the pass");
}

/// The guard's path (every write reads its pictures' analyses, `Settings::guard`, with the hashes it compares taken from
/// the join's store) makes what the unguarded path does, on one slice and on slices of one picture.
#[test]
fn a_guarded_pass_makes_the_pairs_an_unguarded_one_does() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("guarded");
    let plain = pg_import::pairs::pair(&mut c, &opts(), &ctl).unwrap();
    assert_eq!(plain.guarded, 0, "nothing changed since the load");
    let pairs = state::pairs(&c).unwrap();
    for slice in [pg_import::pairs::SLICE, 1] {
        c.pairs_again().unwrap();
        let settings = pg_import::pairs::Settings { slice, guard: true, ..pg_import::pairs::Settings::default() };
        let guarded = pg_import::pairs::pair_with(&mut c, &opts(), settings, &ctl).unwrap();
        assert!(guarded.complete);
        assert!(guarded.guarded > 0);
        assert_eq!((guarded.files, guarded.pairs), (plain.files, plain.pairs));
        assert_eq!(state::pairs(&c).unwrap(), pairs);
        assert_eq!(c.owed_pairs().unwrap(), 0);
    }
}

/// A write that fails while the next slice's join runs (slices of one picture, so every write overlaps a join) is the
/// pass's error: the join is stopped and joined, nothing of the batch is stored, and the next pass makes it all.
#[test]
fn a_write_that_fails_while_the_next_join_runs_stops_it_and_is_the_error() {
    let _one = ONE_PASS.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut c, ctl, _dir) = imported("overlap-fault");
    let owed = c.owed_pairs().unwrap();
    assert!(owed >= 2, "a second slice to join while the first is written");
    let settings = pg_import::pairs::Settings { slice: 1, ..pg_import::pairs::Settings::default() };
    state::fail_pair_writes(&c).unwrap();
    assert!(pg_import::pairs::pair_with(&mut c, &opts(), settings, &ctl).is_err());
    assert!(state::pairs(&c).unwrap().is_empty());
    assert_eq!(state::cache_kib(&c).unwrap(), 8_192, "the catalog's own cache, after a failed pass");
    state::allow_pair_writes(&c).unwrap();
    let made = pg_import::pairs::pair_with(&mut c, &opts(), settings, &ctl).unwrap();
    assert!(made.complete);
    let (lo, hi) = the_twins(&c);
    assert!(state::pairs(&c).unwrap().contains(&(lo, hi, 0)));
    assert_eq!(c.owed_pairs().unwrap(), 0);
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
