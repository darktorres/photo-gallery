//! A long browsing session's saves keep the catalog's WAL bounded. A session save is a write that no round or drain
//! checkpoints for, so `save_listing` checkpoints once the WAL is over its bound (section 11). Without that bound, 3,000
//! saves would leave about 12 MB of WAL.

use pg_app::{App, Config};
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::rules::Merged;
use pg_model::view::Listing;
use strum::IntoEnumIterator;

#[test]
fn session_saves_keep_the_wal_near_its_bound() {
    let dir = pg_testkit::run_folder("scratch_app", "session-wal").unwrap();
    let app = App::open(Config::in_home(&dir)).unwrap();
    let catalog = pg_library::layout::Layout::in_home(dir.clone()).catalog_path();
    let mut wal = catalog.into_os_string();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    let key = SortKey::iter().next().unwrap();
    for i in 0..3000 {
        let listing = Listing {
            folder: Merged::from(vec![Name::from(format!("folder {i}").as_str())]),
            recursive: i % 2 == 0,
            sort: Sort { key, descending: i % 3 == 0 },
            query: Query { name: format!("name {i}"), ..Query::default() },
        };
        app.views().save_listing(&listing).unwrap();
    }
    let wal_bytes = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    app.shutdown();
    drop(app);
    // The bound is `views::WAL_CHECKPOINT_OVER` (4 MiB), which the crate keeps private; a checkpoint runs on the save that
    // passes it, so the file stays near that size. A MiB of slack covers the save that crossed it.
    assert!(
        wal_bytes <= (4 << 20) + (1 << 20),
        "the WAL stayed at {wal_bytes} bytes after 3,000 session saves"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
