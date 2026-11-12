//! The saved listing reads back as it was saved (S-18.2): for every folder, recursion, sort and query, what
//! `save_listing` writes, `listing` returns. The session is where the listing lives between starts.

use pg_app::{App, Config};
use pg_model::name::Name;
use pg_model::query::{Query, Sort, SortKey};
use pg_model::rules::Merged;
use pg_model::view::Listing;
use proptest::prelude::*;
use strum::IntoEnumIterator;

/// A merged folder: any number of parts, text and the names a text can't hold (a lone surrogate, a raw byte).
fn folder() -> impl Strategy<Value = Merged> {
    let part = prop_oneof![
        "[a-zA-Z0-9Σé ]{1,8}".prop_map(|s| Name::from(s.as_str())),
        Just(Name::from_wide(&[0x61, 0xD800])),
        Just(Name::from_raw(b"x\xFF")),
    ];
    prop::collection::vec(part, 0..4).prop_map(Merged::from)
}

/// A fresh folder for one case, so every case starts with no catalog: a run folder of its own.
#[expect(clippy::unwrap_used, reason = "a test helper: a folder that cannot be made fails the test")]
fn fresh_dir() -> std::path::PathBuf {
    pg_testkit::run_folder("scratch_listing", "case").unwrap()
}

proptest! {
    // A few cases: each one opens a catalog, and saves and reads back the listing once per sort.
    #![proptest_config(ProptestConfig::with_cases(12))]

    // Every sort key, both ways, in every case: a key left out of a generator would not be covered at all.
    #[test]
    fn the_saved_listing_reads_back(
        folder in folder(),
        recursive in any::<bool>(),
        name in "[a-zA-Z ]{0,8}",
        exts in prop::collection::vec("[a-z]{1,4}", 0..3),
    ) {
        let dir = fresh_dir();
        let app = App::open(Config::in_home(&dir))
        .unwrap();
        let mut read = Vec::new();
        for key in SortKey::iter() {
            for descending in [false, true] {
                let listing = Listing {
                    folder: folder.clone(),
                    recursive,
                    sort: Sort { key, descending },
                    query: Query { name: name.clone(), exts: exts.clone(), ..Query::default() },
                };
                app.views().save_listing(&listing).unwrap();
                read.push((app.views().listing().unwrap(), listing));
            }
        }
        app.shutdown();
        drop(app);
        // After the shutdown nothing holds the app's files.
        prop_assert!(std::fs::remove_dir_all(&dir).is_ok() && !dir.exists(), "{} is still held", dir.display());
        for (back, listing) in read {
            prop_assert_eq!(back, listing);
        }
    }
}
