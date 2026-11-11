//! The borders pass on real files: a file that can't be read stays due for the next pass, so it is looked at again once
//! it can be read. Another program holds the file (pg_testkit::held): on Windows it is open without sharing, on Unix it
//! has no read permission, and the hold lets go before the next pass. A file that doesn't decode is recorded in the
//! failure ledger, and leaves it once its borders are found.

use std::path::Path;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_catalog::ledger;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::notice::FilePass;
use pg_testkit::held::{Access, hold};

#[test]
fn an_unreadable_file_stays_due_for_the_next_pass() {
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
    assert!(ledger::recorded(&c, FilePass::Borders).unwrap().is_empty());
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
    let [failed] = &ledger::files(&c, FilePass::Borders, 10).unwrap()[..] else { panic!("one failure") };
    assert_eq!((failed.file_id, failed.retry), (state::file_id(&c, "a.png").unwrap(), false));
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 0, "not looked at again");
    // Mended, and due again: found, and gone from the ledger.
    rewrite(&good);
    state::make_borders_due(&c, "a.png").unwrap();
    let stats = pg_borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed), (1, 0));
    assert!(ledger::recorded(&c, FilePass::Borders).unwrap().is_empty());
    drop(c);
    let _ = std::fs::remove_dir_all(Path::new(&dir));
}
