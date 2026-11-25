//! A write run whose writer ends: the run ends with the writer's stop, and the changes stay queued. These tests need the `fault`
//! feature (`cargo test -p pg-driver --features fault`), which lets the writer fail where a test says.

#![cfg(feature = "fault")]

use std::time::Duration;

use pg_catalog::Catalog;
use pg_driver::{Error, Headless, Setup};
use pg_library::layout::Layout;
use pg_library::mask::PassMask;
use pg_model::turns::Turns;
use pg_platform::native::PATH_RULES;
use pg_writeback::fault::Fault;

/// How long a run may take in a test: generous, so a loaded machine does not fail one.
const WAIT: Duration = Duration::from_secs(120);

#[test]
fn a_writer_that_ends_ends_the_write_run_with_the_changes_still_queued() {
    let dir = pg_testkit::run_folder_with("pg_driver_tests", "writer-ends", &["home", "images"]).unwrap();
    let images = dir.join("images");
    let home = dir.join("home");
    std::fs::write(images.join("a.png"), pg_codec::fixtures::png_rgb(8, 8, &[0; 192]).unwrap()).unwrap();

    // The writer ends at its first drain of the queue, before any write is attempted.
    let mut writing = Setup::new(&home);
    writing.create = true;
    writing.writer = true;
    writing.writer_fault = Some(Fault::PanicInDrain);
    let headless = Headless::open(&writing).unwrap();
    headless.add_folder(&images).unwrap();
    headless.run(PassMask::ALL, WAIT).unwrap();

    let catalog = Catalog::open(&Layout::in_home(&home).catalog_path(), PATH_RULES).unwrap();
    let folder = PATH_RULES.parse(&images.to_string_lossy()).unwrap();
    let file = catalog.files_under(&folder).unwrap()[0];
    catalog.queue_rotation(file, Turns::new(1)).unwrap();

    let result = headless.write_pending(WAIT);
    assert!(matches!(result, Err(Error::WriterStopped { .. })), "{result:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
