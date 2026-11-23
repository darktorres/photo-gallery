//! The Hashes pass on real files: an import with duplicates off stores no hashes and owes them (`hashes_owed`), and makes
//! no pairs; the pass then makes the hashes an import with duplicates on stores, and the pairing finds the same pairs.

use std::path::PathBuf;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::notice::FilePass;
use pg_platform::priority::Priority;

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A run folder with `images/a.png` and `images/b.png` (the same textured picture) and `images/c.png` (another one),
/// scanned into a catalog beside them, not imported: the catalog, the run's control, and the folder.
fn scanned(prefix: &str) -> Result<(Catalog, Control, PathBuf)> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    let images = dir.join("images");
    std::fs::create_dir_all(&images)?;
    let png = pg_codec::fixtures::Encoding::Png;
    let same = pg_codec::fixtures::encode(96, 80, &pg_testkit::textured_rgb(96, 80), png)?;
    std::fs::write(images.join("a.png"), &same)?;
    std::fs::write(images.join("b.png"), &same)?;
    std::fs::write(images.join("c.png"), pg_codec::fixtures::encode(96, 80, &pg_testkit::other_rgb(96, 80), png)?)?;
    let mut c = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), pg_platform::native::PATH_RULES)?;
    let ctl = Control::new();
    let root = pg_platform::names::normalize_path(&images)?;
    c.add_root(&root)?;
    pg_scan::scan(&mut c, &root, &ctl)?;
    Ok((c, ctl, dir))
}

fn opts() -> Options {
    Options::new(Priority::Normal)
}

#[test]
fn the_hashes_pass_makes_what_an_import_with_them_would_have() -> Result<()> {
    let (mut with, ctl, _with_dir) = scanned("with")?;
    pg_import::import(&mut with, &opts(), false, &pg_catalog::VolumeSet::All, &ctl).unwrap();
    pg_import::hashes::hash(&mut with, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    pg_import::pairs::pair(&mut with, &opts(), &ctl).unwrap();

    let (mut without, ctl, _without_dir) = scanned("without")?;
    pg_import::import_while(&mut without, &opts(), false, &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(without.owed_hashes().unwrap(), 3, "every picture owes its hashes");
    assert!(state::stored_hashes(&without).unwrap().iter().all(|(_, file, pixel)| file.is_none() && pixel.is_none()));
    assert!(state::pairs(&without).unwrap().is_empty(), "no hashes, no pairs");

    let stats = pg_import::hashes::hash(&mut without, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    assert_eq!(stats.stored, 3);
    assert_eq!(stats.failed, 0);
    assert_eq!(without.owed_hashes().unwrap(), 0, "the pass cleared what it owed");
    pg_import::pairs::pair(&mut without, &opts(), &ctl).unwrap();

    assert_eq!(state::stored_hashes(&without).unwrap(), state::stored_hashes(&with).unwrap());
    let pairs = state::pairs(&with).unwrap();
    assert!(!pairs.is_empty(), "the same picture twice is a pair");
    assert_eq!(state::pairs(&without).unwrap(), pairs);
    Ok(())
}

/// A file the hashes can't be read from stays owed, and is said (`unreadable`), not recorded in the failures; a file that
/// is read but isn't a picture becomes an import failure at once, and its owed row goes with it.
#[test]
fn an_unreadable_file_stays_owed_and_an_undecodable_one_fails_at_once() -> Result<()> {
    let (mut c, ctl, dir) = scanned("failures")?;
    pg_import::import_while(&mut c, &opts(), false, &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    // a.png is gone from the disk (the catalog does not know yet): it can't be read, so it stays owed.
    std::fs::remove_file(dir.join("images").join("a.png")).unwrap();
    // b.png is read, but it is no picture: its hashes fail, and it is an import failure at once.
    std::fs::write(dir.join("images").join("b.png"), b"not a picture").unwrap();
    let stats = pg_import::hashes::hash(&mut c, &opts(), &pg_catalog::VolumeSet::All, &|| true, &ctl).unwrap();
    // `failed` counts the unreadable file too (`Stats`: `unreadable` is the part of `failed` that couldn't be read).
    assert_eq!((stats.stored, stats.unreadable, stats.failed), (1, 1, 2));
    assert_eq!(c.owed_hashes().unwrap(), 1, "only the unreadable file is owed");
    let failures = c.failed_files(FilePass::Import, 10).unwrap();
    assert_eq!(failures.len(), 1, "the undecodable file is an import failure; the unreadable one is in no ledger");
    Ok(())
}
