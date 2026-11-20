//! What `Catalog::prepare` does with a file, decided from the facts it reads of it (`pure`, no I/O). The decisions are the
//! refusals first: a catalog this build cannot open is refused before anything is written, and a released catalog's
//! drift is never rebuilt (the carry-rebuild is for development only, while nothing is released).

/// The catalog schema versions that have been released. Empty until the first release: every version is then the
/// development one, and a catalog whose schema drifted from this build's may be rebuilt with its rows carried over.
/// Appending a version is part of the release (`RELEASED`'s checklist, with the snapshot of that version's schema).
pub(crate) const RELEASED: &[i64] = &[];

/// What is known of a file for [`classify`].
pub(crate) struct Facts<'a> {
    /// The file's `user_version`, or `None` when the file is not a database.
    pub(crate) version: Option<i64>,
    /// The version this build's schema makes.
    pub(crate) current: i64,
    /// The schema difference from the expected schema for `version` (`None`: it is as expected).
    pub(crate) drift: Option<&'a str>,
    /// Whether the file has a `key_version` mark: a rebuild carries the rows, and the carry needs to know their keys.
    pub(crate) has_key_version: bool,
}

/// What `prepare` does with the file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Not a database: moved aside, and a new catalog is made.
    MoveAsideThenCreate,
    /// An empty file: made.
    Create,
    /// This build's schema, as expected: opened (the caller decides what else it owes).
    Open,
    /// An earlier version of this build's schema: migrated forward.
    Upgrade { from: i64 },
    /// A schema that differs from this build's: rebuilt, its rows carried over (development only).
    RebuildCarry { why: String },
    /// Not opened: the reason is the refusal.
    Refuse(Refusal),
}

/// Why a file is refused. Nothing is written to a refused file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A version after this build's: made by another build.
    NewerSchema { found: i64, current: i64 },
    /// A schema that differs from this build's, of a catalog that may not be rebuilt (see [`RELEASED`]).
    Drift { found: i64, why: String },
    /// A schema that differs, of a catalog without the `key_version` mark the carry needs.
    PredatesKeyVersion,
}

/// The action for a file with `facts`, given the released schema versions `released`.
pub(crate) fn classify(facts: &Facts<'_>, released: &[i64]) -> Action {
    let Some(found) = facts.version else {
        return Action::MoveAsideThenCreate;
    };
    if found > facts.current {
        return Action::Refuse(Refusal::NewerSchema { found, current: facts.current });
    }
    if found == 0 {
        return Action::Create;
    }
    if let Some(why) = facts.drift {
        if !facts.has_key_version {
            return Action::Refuse(Refusal::PredatesKeyVersion);
        }
        // Once anything is released, a catalog that drifted from its version's schema is never rebuilt: a drift at or below
        // the newest released version is a change to a released schema, and above it a change that was not versioned.
        if !released.is_empty() {
            return Action::Refuse(Refusal::Drift { found, why: why.to_string() });
        }
        return Action::RebuildCarry { why: why.to_string() };
    }
    if found < facts.current {
        return Action::Upgrade { from: found };
    }
    Action::Open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(version: Option<i64>, drift: Option<&str>, has_key_version: bool) -> Facts<'_> {
        Facts { version, current: 3, drift, has_key_version }
    }

    #[test]
    fn a_file_that_is_not_a_database_is_moved_aside_and_made_anew() {
        assert_eq!(classify(&facts(None, None, false), &[]), Action::MoveAsideThenCreate);
    }

    #[test]
    fn an_empty_file_is_made() {
        assert_eq!(classify(&facts(Some(0), None, false), &[]), Action::Create);
    }

    #[test]
    fn a_later_build_schema_is_refused_whatever_else_differs() {
        assert_eq!(
            classify(&facts(Some(4), Some("a table"), true), &[]),
            Action::Refuse(Refusal::NewerSchema { found: 4, current: 3 })
        );
        assert_eq!(
            classify(&facts(Some(4), None, true), &[]),
            Action::Refuse(Refusal::NewerSchema { found: 4, current: 3 })
        );
    }

    #[test]
    fn the_schema_as_expected_is_opened_and_an_earlier_one_is_upgraded() {
        assert_eq!(classify(&facts(Some(3), None, true), &[]), Action::Open);
        assert_eq!(classify(&facts(Some(2), None, true), &[]), Action::Upgrade { from: 2 });
    }

    #[test]
    fn a_drifted_schema_is_carried_over_while_nothing_is_released() {
        assert_eq!(
            classify(&facts(Some(3), Some("an index"), true), &[]),
            Action::RebuildCarry { why: "an index".to_string() }
        );
    }

    #[test]
    fn a_drifted_schema_without_the_key_version_mark_is_refused_before_any_carry() {
        assert_eq!(
            classify(&facts(Some(3), Some("an index"), false), &[]),
            Action::Refuse(Refusal::PredatesKeyVersion)
        );
    }

    #[test]
    fn once_released_a_drifted_schema_is_refused_not_carried() {
        for found in [1, 2, 3] {
            let facts = Facts { version: Some(found), current: 3, drift: Some("an index"), has_key_version: true };
            assert_eq!(
                classify(&facts, &[1]),
                Action::Refuse(Refusal::Drift { found, why: "an index".to_string() }),
                "version {found}"
            );
        }
    }

    #[test]
    fn nothing_is_released_in_a_development_build() {
        // The version string says so, and the release checklist changes both together (`RELEASED` and `-dev`).
        assert_eq!(RELEASED.is_empty(), env!("CARGO_PKG_VERSION").ends_with("-dev"));
    }
}

/// The decisions as `Catalog::prepare` makes them, on real files.
#[cfg(test)]
mod prepare_tests {
    use crate::Catalog;
    use crate::test_support::versions;
    use pg_model::rules::PathRules;
    use pg_testkit::run_folder;
    use rusqlite::Connection;
    use std::path::Path;

    fn meta(path: &Path, key: &str) -> String {
        Connection::open(path).unwrap().query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).unwrap()
    }

    /// The catalog a build changes says which build did; a catalog that is only opened is not written to.
    #[test]
    fn the_build_that_changed_the_catalog_is_named_in_it() {
        let dir = run_folder("scratch_catalog", "written-by").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        assert_eq!(meta(&path, "written_by"), env!("CARGO_PKG_VERSION"));
        // Opened again with nothing to change: the stamp of the build that made it is kept, and the file is not written.
        Connection::open(&path)
            .unwrap()
            .execute("UPDATE meta SET value = 'an-older-build' WHERE key = 'written_by'", [])
            .unwrap();
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        assert_eq!(meta(&path, "written_by"), "an-older-build");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A schema after this build's is refused by the build that made it, named, and nothing in the file is changed.
    #[test]
    fn a_later_schema_is_refused_by_name_and_left_as_it_is() {
        let dir = run_folder("scratch_catalog", "later-schema").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        let later = crate::schema::current_version().unwrap() + 1;
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = 'a-later-build' WHERE key = 'written_by'", []).unwrap();
        conn.pragma_update(None, "user_version", later).unwrap();
        drop(conn);
        let err = format!("{:#}", Catalog::prepare(&path, versions(), PathRules::Windows).err().expect("refused"));
        assert!(err.contains("made by another build by build a-later-build"), "{err}");
        assert!(err.contains("restore a backup by hand"), "{err}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), later);
        drop(conn);
        assert_eq!(meta(&path, "written_by"), "a-later-build");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
