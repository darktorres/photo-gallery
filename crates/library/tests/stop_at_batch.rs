//! The passes that a toggle or a zoom can end: each one ends after the batch in which its `still_wanted` turns false, and
//! what it did not reach stays owed (or pending, or due) for the next run. Each test has more files than one batch
//! (`N`), so a stop has work left to show.

use std::path::PathBuf;

use pg_catalog::Catalog;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_platform::priority::Priority;

/// More files than one batch of the import, the hashes and the borders (500 each): a stop after the first batch leaves
/// some. If a batch grows past this, the tests fail, and this number is the one to raise.
const N: usize = 501;

/// A run folder with `N` pictures of 64 × 64 (the border detector looks at those), scanned into a catalog beside them,
/// not imported: the catalog, the run's control, and the folder.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn scanned(prefix: &str) -> (Catalog, Control, PathBuf) {
    let dir = pg_testkit::run_folder("scratch_library", prefix).unwrap();
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    // A pattern, not a flat colour: the picture is a real one for every pass.
    let pixels: Vec<u8> = (0..64 * 64 * 3).map(|i| u8::try_from(i % 251).unwrap()).collect();
    let png = pg_testkit::png_rgb(64, 64, &pixels).unwrap();
    for i in 0..N {
        std::fs::write(images.join(format!("{i:04}.png")), &png).unwrap();
    }
    let mut c =
        Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images).unwrap();
    c.add_root(&root).unwrap();
    pg_scan::scan(&mut c, &root, &ctl).unwrap();
    (c, ctl, dir)
}

fn opts() -> Options {
    Options::new(Priority::Normal)
}

#[test]
fn an_import_stops_after_its_batch_and_leaves_the_rest_pending() {
    let (mut c, ctl, _dir) = scanned("import");
    let stats = pg_import::import_while(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(stats.files > 0 && stats.files < N, "one batch was imported: {}", stats.files);
    let rest = c.pending_import(false, &pg_catalog::VolumeSet::All).unwrap().items.len();
    assert_eq!(stats.files + rest, N, "the files not reached stay pending");
    pg_import::import_while(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(
        c.pending_import(false, &pg_catalog::VolumeSet::All).unwrap().items.is_empty(),
        "the next run imports the rest"
    );
}

#[test]
fn the_hashes_pass_stops_after_its_batch_and_keeps_the_rest_owed() {
    let (mut c, ctl, _dir) = scanned("hashes");
    pg_import::import_while(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(c.owed_hashes().unwrap(), u64::try_from(N).unwrap(), "every picture owes its hashes");
    let stats = pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(!stats.complete, "a stopped pass is not complete");
    assert!(stats.stored > 0 && stats.stored < N, "one batch was hashed: {}", stats.stored);
    assert_eq!(c.owed_hashes().unwrap(), u64::try_from(N - stats.stored).unwrap(), "the files not reached stay owed");
    let done = pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(done.complete);
    assert_eq!(c.owed_hashes().unwrap(), 0, "the next run hashes the rest");
}

#[test]
fn the_borders_pass_stops_after_its_batch_and_keeps_the_rest_due() {
    let (mut c, ctl, _dir) = scanned("borders");
    pg_import::import_while(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(
        c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.len(),
        N,
        "every picture is due its borders"
    );
    let stats = pg_borders::find_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| false, &ctl).unwrap();
    assert!(!stats.complete, "a stopped pass is not complete");
    let rest = c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.len();
    assert!(rest > 0 && rest < N, "one batch was looked at; {rest} are still due");
    pg_borders::find_while(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert!(c.pending_borders(&pg_catalog::VolumeSet::All).unwrap().items.is_empty(), "the next run looks at the rest");
}
