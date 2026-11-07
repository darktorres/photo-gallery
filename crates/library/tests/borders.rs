//! The borders pass on real files: a file that can't be read stays due for the next pass, so it is looked at again once
//! it can be read. Another program holds the file (pg_testkit::held): on Windows it is open without sharing, on Unix it
//! has no read permission, and the hold lets go before the next pass.

use std::path::Path;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_library::borders;
use pg_model::rules::PathRules;
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
    let mut c = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&images).unwrap();
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
    let stats = borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!(stats.failed, 1);
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 1, "an unreadable file must stay due");
    assert_eq!(state::borders_due(&c, "b.png").unwrap(), 0, "the readable one is done");

    // Readable again: the next pass looks at it, and it is no longer due.
    drop(lock);
    let stats = borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!(stats.failed, 0);
    assert_eq!(state::borders_due(&c, "a.png").unwrap(), 0);

    drop(c);
    let _ = std::fs::remove_dir_all(Path::new(&images));
}
