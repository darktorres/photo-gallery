//! The borders pass on real files: a file that can't be read stays due for the next pass (R-47), so it is looked at
//! again once it can be read. It locks the file by a sharing mode, which only Windows has, so the file is Windows only.
#![cfg(windows)]

use std::path::Path;

use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_library::borders;
use pg_model::rules::PathRules;

#[cfg(windows)]
#[test]
fn an_unreadable_file_stays_due_for_the_next_pass() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_borders", "borders");
    let images = dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbImage::from_fn(96, 80, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, 128]))
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
    assert_eq!((state::borders_due(&c, "a.png"), state::borders_due(&c, "b.png")), (1, 1));

    // a.png is locked: the pass can't read it.
    let lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(images.join("a.png")).unwrap();
    let stats = borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!(stats.failed, 1);
    assert_eq!(state::borders_due(&c, "a.png"), 1, "an unreadable file must stay due");
    assert_eq!(state::borders_due(&c, "b.png"), 0, "the readable one is done");

    // Readable again: the next pass looks at it, and it is no longer due.
    drop(lock);
    let stats = borders::find(&mut c, &opts, &ctl).unwrap();
    assert_eq!(stats.failed, 0);
    assert_eq!(state::borders_due(&c, "a.png"), 0);

    drop(c);
    let _ = std::fs::remove_dir_all(Path::new(&images));
}
