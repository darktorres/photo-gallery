//! What `Catalog::prepare` does with a file, decided from the facts it reads of it (`pure`, no I/O). The decisions are the
//! refusals first: a catalog this build cannot open is refused before anything is written, and a released catalog's
//! drift is never rebuilt (the carry-rebuild is for development only, while nothing is released).

use crate::error::Refusal;
use crate::lifecycle::NewerMark;

/// The catalog schema versions that have been released. Empty until the first release: every version is then the
/// development one, and a catalog whose schema drifted from this build's may be rebuilt with its rows carried over.
/// Appending a version is part of the release (`RELEASED`'s checklist, with the snapshot of that version's schema).
pub(crate) const RELEASED: &[i64] = &[];

/// The longest name the catalog writes beside its file: a backup's partial copy's journal, at the plan's widest numbers
/// (`.pre-v<N>-<seq>.bak.partial-journal`). `prepare` refuses a path that cannot hold it on Windows (`fits_beside`).
pub(crate) const LONGEST_NAME_SUFFIX: &str = ".pre-v99-999999.bak.partial-journal";

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
    /// An integer mark above this build's, which a later build set (`lifecycle::newer_mark`): refused as a newer schema is.
    pub(crate) newer_mark: Option<NewerMark>,
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

/// The action for a file with `facts`, given the released schema versions `released`.
pub(crate) fn classify(facts: &Facts<'_>, released: &[i64]) -> Action {
    let Some(found) = facts.version else {
        return Action::MoveAsideThenCreate;
    };
    if found > facts.current {
        return Action::Refuse(Refusal::NewerSchema { found, current: facts.current, written_by: None });
    }
    if let Some(mark) = facts.newer_mark {
        return Action::Refuse(Refusal::NewerMark {
            mark: mark.name,
            found: mark.found,
            current: mark.current,
            written_by: None,
        });
    }
    // A file of version 0 with objects in it is a foreign or damaged database, not an empty one: its drift is handled
    // as any other's (it has no rows, so it needs no `key_version` to say how they are keyed).
    if let Some(why) = facts.drift {
        if found > 0 && !facts.has_key_version {
            return Action::Refuse(Refusal::PredatesKeyVersion);
        }
        // Once anything is released, a catalog that drifted from its version's schema is never rebuilt: a drift at or below
        // the newest released version is a change to a released schema, and above it a change that was not versioned.
        if !released.is_empty() {
            return Action::Refuse(Refusal::Drift { found, why: why.to_string() });
        }
        return Action::RebuildCarry { why: why.to_string() };
    }
    if found == 0 {
        return Action::Create;
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
        Facts { version, current: 3, drift, has_key_version, newer_mark: None }
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
            Action::Refuse(Refusal::NewerSchema { found: 4, current: 3, written_by: None })
        );
        assert_eq!(
            classify(&facts(Some(4), None, true), &[]),
            Action::Refuse(Refusal::NewerSchema { found: 4, current: 3, written_by: None })
        );
    }

    #[test]
    fn a_later_build_mark_is_refused_as_a_newer_schema_is() {
        let mark = NewerMark { name: "import_version", found: 5, current: 4 };
        let facts = Facts { newer_mark: Some(mark), ..facts(Some(3), None, true) };
        assert_eq!(
            classify(&facts, &[]),
            Action::Refuse(Refusal::NewerMark { mark: "import_version", found: 5, current: 4, written_by: None })
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
            let facts = Facts {
                version: Some(found),
                current: 3,
                drift: Some("an index"),
                has_key_version: true,
                newer_mark: None,
            };
            assert_eq!(
                classify(&facts, &[1]),
                Action::Refuse(Refusal::Drift { found, why: "an index".to_string() }),
                "version {found}"
            );
        }
    }

    /// Every combination of the facts, each against the action its rule gives. The impossible ones are left out: a drift
    /// needs a database (`version` is `Some`), and a later build's schema has no expected schema to drift from, so
    /// versions above `CURRENT` have no drift. That leaves 10 (version, drift) pairs, each with 2 `key_version` states and
    /// 2 release states: 40 cases.
    #[test]
    fn every_possible_combination_of_facts_gets_its_rule_s_action() {
        const CURRENT: i64 = 3;
        const DRIFT: &str = "a difference";
        let pairs = [
            (None, None),
            (Some(0), None),
            (Some(0), Some(DRIFT)),
            (Some(1), None),
            (Some(1), Some(DRIFT)),
            (Some(2), None),
            (Some(2), Some(DRIFT)),
            (Some(3), None),
            (Some(3), Some(DRIFT)),
            (Some(4), None),
        ];
        let mut checked = 0;
        for (version, drift) in pairs {
            for has_key_version in [false, true] {
                for released in [&[][..], &[2][..]] {
                    let facts = Facts { version, current: CURRENT, drift, has_key_version, newer_mark: None };
                    let expected = match (version, drift) {
                        (None, _) => Action::MoveAsideThenCreate,
                        (Some(found), _) if found > CURRENT => {
                            Action::Refuse(Refusal::NewerSchema { found, current: CURRENT, written_by: None })
                        }
                        (Some(found), Some(_)) if found > 0 && !has_key_version => {
                            Action::Refuse(Refusal::PredatesKeyVersion)
                        }
                        (Some(found), Some(why)) if !released.is_empty() => {
                            Action::Refuse(Refusal::Drift { found, why: why.to_string() })
                        }
                        (Some(_), Some(why)) => Action::RebuildCarry { why: why.to_string() },
                        (Some(0), None) => Action::Create,
                        (Some(found), None) if found < CURRENT => Action::Upgrade { from: found },
                        (Some(_), None) => Action::Open,
                    };
                    assert_eq!(
                        classify(&facts, released),
                        expected,
                        "{version:?} {drift:?} key {has_key_version} released {released:?}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 40);
    }

    /// The longest name the catalog makes beside its file is the one `fits_beside` reserves room for.
    #[test]
    fn the_longest_name_is_the_widest_the_catalog_makes() {
        assert_eq!(LONGEST_NAME_SUFFIX.len(), 35, "the plan's widest backup name, sequence and version included");
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
    use crate::error::Refusal;
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
        let _ = std::fs::remove_dir_all(&dir);
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A catalog that needs a change and is read-only is refused by name, and the file is left as it was.
    #[test]
    fn a_read_only_catalog_that_needs_a_change_is_refused_and_left_as_it_is() {
        let dir = run_folder("scratch_catalog", "read-only").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        // A `key_version` another build wrote: a change that `prepare` makes, so the read-only file is refused.
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = 'another-key-algo' WHERE key = 'key_version'", []).unwrap();
        drop(conn);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();

        let err = Catalog::prepare(&path, versions(), PathRules::Windows).err().expect("refused");
        assert_eq!(err.refusal().map(|(_, r)| r), Some(&Refusal::ReadOnly), "{err:#}");
        assert_eq!(meta(&path, "key_version"), "another-key-algo", "nothing of the catalog is changed");

        // The test's own folder is removed below: the file is made writable again first.
        #[expect(
            clippy::permissions_set_readonly_false,
            reason = "a scratch file of this test, restored to remove its folder"
        )]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path too long for the files made beside it is refused by name before anything is made: no lock file, no folder.
    #[cfg(windows)]
    #[test]
    fn a_path_too_long_for_its_files_is_refused_before_anything_is_made() {
        let dir = run_folder("scratch_catalog", "path-too-long").unwrap();
        let folder = dir.join("n".repeat(240));
        let path = folder.join("catalog.sqlite");
        let err = Catalog::prepare(&path, versions(), PathRules::Windows).err().expect("refused");
        let (refused_path, reason) = err.refusal().expect("a typed refusal");
        assert_eq!(reason, &Refusal::PathTooLong);
        assert_eq!(refused_path, path.display().to_string());
        assert!(!folder.exists(), "nothing is made under a path too long for its files");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
