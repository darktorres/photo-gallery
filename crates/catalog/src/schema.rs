//! The catalog's schema: the SQL that makes a new catalog, the version a file is at, and how a file is brought to a
//! version. A file is compared with [`expected_for`], the schema this build makes in memory from the same SQL, so the
//! layout checked is the layout created.
//!
//! ## Storage
//!
//! The layout is [`BASELINE`], a literal SQL file (`baseline.sql`): every table, index, view and trigger the catalog
//! has, and the rows creation seeds in `meta`. Before the first release it is edited in place and the version stays
//! [`BASELINE_VERSION`]; from the release on every change is a migration ([`MIGRATIONS`]) and the baseline is frozen.
//! The ids of `files`, `folders` and `roots` are AUTOINCREMENT, so a removed id is never reissued; `notices` is bounded
//! by its trigger; `file_failures` keeps an import row hidden while its file is pending or missing.
//!
//! A migration is a list of steps ([`Step`]): SQL that changes the layout, or a Rust conversion of rows, which finds the
//! old shape by the columns it has and is idempotent. The rebuild's copy runs every Rust step of every migration on the
//! copied rows (`crate::rebuild::carry_rows`).

use anyhow::{Context, anyhow};
use pg_dbfile::{Damage, Expected, Schema};
use pg_model::rules::PathRules;
use rusqlite::{Connection, Transaction};

use crate::convert::Source;
use crate::error::Result;
use crate::progress::{NO_PROGRESS, Phase, Progress};
use crate::transaction::write_transaction;

/// One step of a migration: SQL that changes the layout, or a conversion of rows (`Source`: the old shape it reads from),
/// which is idempotent. `PathRules`: the rules the catalog's paths are stored under.
#[derive(Clone, Copy)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no migration yet: before the first release the baseline is edited in place")
)]
pub(crate) enum Step {
    Sql(&'static str),
    Rust(fn(&Transaction<'_>, Source, PathRules) -> Result<()>),
}

/// The version of a catalog made from the baseline, before any migration.
pub(crate) const BASELINE_VERSION: i64 = 1;

/// One migration: the steps that take a catalog one version on, in order, in one transaction with the version.
/// `rebuilds_tables`: the migration drops tables that have dependants. `DROP TABLE` of a parent runs an implicit `DELETE`
/// with foreign keys on, which cascades into its children (their analyses, ratings, queued writes), so such a migration
/// runs with foreign keys off and `legacy_alter_table` on (see [`ForeignKeysOff`]), and checks that they are off before
/// its first step. The pragma is a no-op inside a transaction, so it is set around the migration, not in it.
pub(crate) struct Migration {
    pub(crate) steps: &'static [Step],
    pub(crate) rebuilds_tables: bool,
}

/// A baseline and the migrations after it: the layout a catalog has at each version. The build's own is `SCHEMA`.
#[derive(Clone, Copy)]
pub(crate) struct SchemaSet<'a> {
    pub(crate) baseline: &'a str,
    pub(crate) migrations: &'a [Migration],
}

/// The layout a new catalog is made with, at [`BASELINE_VERSION`]: every object, and the seeds of `meta`.
pub(crate) const BASELINE: &str = include_str!("baseline.sql");

/// This build's schema: [`BASELINE`] and [`MIGRATIONS`].
pub(crate) const SCHEMA: SchemaSet<'static> = SchemaSet { baseline: BASELINE, migrations: MIGRATIONS };

/// The migrations after the baseline, in order: `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next. None
/// before the first release, while the baseline is edited in place (see the module docs).
pub(crate) const MIGRATIONS: &[Migration] = &[];

/// What a caller watches while a catalog is brought to a version: the progress its steps are reported to. The fault
/// points come through this same parameter (Phase 4), so the signature of [`bring_to`] changes once.
pub(crate) struct Hooks<'a> {
    pub(crate) progress: &'a dyn Progress,
}

impl Hooks<'static> {
    /// Watches nothing.
    pub(crate) fn none() -> Self {
        Hooks { progress: &NO_PROGRESS }
    }
}

impl<'a> Hooks<'a> {
    /// Reports to `progress`.
    pub(crate) fn reporting(progress: &'a dyn Progress) -> Self {
        Hooks { progress }
    }
}

/// The `application_id` header field of a catalog file (`PGCT`). Set when the file is made, and checked with its
/// schema, so a SQLite file that merely has the same tables is not taken for a catalog.
pub(crate) const APPLICATION_ID: i64 = 0x5047_4354;

/// The rows that creation seeds and that a catalog must have the keys of: `meta`'s. Their values differ per catalog
/// (its id, the marks it was made with), so only the keys are compared.
const SEEDS: &[(&str, &str)] = &[("meta", "key")];

/// Every connection's settings. `busy_timeout`: a statement that finds the catalog locked by another connection (the
/// background job's writer, a scan) waits up to this long for it, instead of failing at once with "database is locked".
/// `cache_size` (8 MB) is the writers' and the thumbs' cache: the writes touch a batch's rows and their index pages, and
/// keep no more. The grid reader raises its own (`Catalog::set_cache_kib`), since its reads touch most of the catalog.
pub(crate) const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA cache_size = -8192; PRAGMA temp_store = MEMORY; PRAGMA busy_timeout = 5000;";

/// The version this build's catalogs are at: the baseline, then each migration.
pub(crate) fn current_version() -> Result<i64> {
    version_in(&SCHEMA)
}

/// The version the schema `set` brings a catalog to: the baseline, then each of its migrations.
pub(crate) fn version_in(set: &SchemaSet<'_>) -> Result<i64> {
    let migrations = i64::try_from(set.migrations.len()).context("too many migrations")?;
    Ok(BASELINE_VERSION + migrations)
}

/// The version of the database at `conn`, or `None` when the file is not a database (SQLite can't read its header).
pub(crate) fn version_of(conn: &Connection) -> Result<Option<i64>> {
    match pg_dbfile::user_version(conn) {
        Ok(version) => Ok(Some(version)),
        Err(e) if Damage::of(&e) == Some(Damage::NotADatabase) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Bring the database at `conn` up to `target`, each step in its own write transaction. A new database (version 0)
/// gets `baseline`, with its `path_rules` row for `rules`, the `application_id` and [`BASELINE_VERSION`]; a database at
/// version `v` gets `migrations[v - BASELINE_VERSION]`, which takes it to `v + 1`: its steps in order (a `Rust` step with
/// [`Source::Main`]), with the version, in one transaction. Each migration reads the version inside its transaction, so
/// one another connection already made is not made again. A database at or past `target` is left alone; a failed step
/// rolls back its migration and leaves the database at the last good version.
pub(crate) fn bring_to(
    conn: &Connection,
    set: &SchemaSet<'_>,
    rules: PathRules,
    target: i64,
    hooks: &Hooks<'_>,
) -> Result<()> {
    loop {
        // The version is read outside the transaction first, so a migration that needs foreign keys off has them off
        // before its transaction begins: the pragma does nothing inside one. It is read again inside it, and when it has
        // moved (another connection migrated meanwhile) this round starts over.
        let advisory = version_of(conn)?.unwrap_or(0);
        let _foreign_keys_off =
            (advisory < target && rebuilds_tables(set, advisory)).then(|| ForeignKeysOff::new(conn)).transpose()?;
        let tx = write_transaction(conn)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= target {
            return Ok(());
        }
        if version != advisory {
            drop(tx);
            continue;
        }
        if version == 0 {
            tx.execute_batch(set.baseline).context("creating the catalog")?;
            tx.execute("INSERT INTO meta(key, value) VALUES ('path_rules', ?1)", [crate::types::rules_name(rules)])?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", BASELINE_VERSION)?;
            // A file made without incremental auto-vacuum is compacted by a VACUUM that the next start runs (`vacuum_owed`):
            // its creator sets the pragma before the schema, and SQLite ignores it once a page exists.
            let mode: i64 = tx.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
            if mode != 2 {
                tx.execute("UPDATE meta SET value = 1 WHERE key = 'vacuum_owed'", [])?;
            }
        } else {
            let index = usize::try_from(version - BASELINE_VERSION)
                .with_context(|| format!("schema version {version}, older than the baseline {BASELINE_VERSION}"))?;
            let migration = set
                .migrations
                .get(index)
                .ok_or_else(|| anyhow!("schema version {version}: no migration takes it on"))?;
            if migration.rebuilds_tables {
                require_foreign_keys_off(&tx)?;
            }
            let context = || format!("migration to schema version {}", version + 1);
            for (step_index, step) in migration.steps.iter().enumerate() {
                hooks.progress.phase(Phase::Migrate {
                    from: version,
                    to: version + 1,
                    step: step_index,
                    steps: migration.steps.len(),
                });
                match *step {
                    Step::Sql(sql) => tx.execute_batch(sql).with_context(context)?,
                    Step::Rust(convert) => convert(&tx, Source::Main, rules).with_context(context)?,
                }
            }
            tx.pragma_update(None, "user_version", version + 1)?;
        }
        tx.commit()?;
    }
}

/// Foreign keys off and `legacy_alter_table` on, for a migration that needs them ([`Migration::rebuilds_tables`]). The
/// connection's own values are read first and put back when the guard drops, whether the migration committed or not, so a
/// later migration on the connection runs as it would have. `legacy_alter_table` on keeps a rename from re-resolving the
/// views and triggers of the other tables, which name a table that is missing between the drop and the rename.
struct ForeignKeysOff<'c> {
    conn: &'c Connection,
    foreign_keys: bool,
    legacy_alter_table: bool,
}

impl<'c> ForeignKeysOff<'c> {
    fn new(conn: &'c Connection) -> Result<Self> {
        let guard = Self {
            conn,
            foreign_keys: conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))? != 0,
            legacy_alter_table: conn.query_row("PRAGMA legacy_alter_table", [], |r| r.get::<_, i64>(0))? != 0,
        };
        conn.pragma_update(None, "foreign_keys", false)?;
        conn.pragma_update(None, "legacy_alter_table", true)?;
        Ok(guard)
    }
}

impl Drop for ForeignKeysOff<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.conn.pragma_update(None, "legacy_alter_table", self.legacy_alter_table) {
            pg_log::warn("catalog", &format!("putting back the connection's legacy_alter_table: {e}"));
        }
        if let Err(e) = self.conn.pragma_update(None, "foreign_keys", self.foreign_keys) {
            pg_log::warn("catalog", &format!("putting back the connection's foreign_keys: {e}"));
        }
    }
}

/// Whether the migration that takes `version` on drops tables that have dependants ([`Migration::rebuilds_tables`]). A
/// version before the baseline has none.
fn rebuilds_tables(set: &SchemaSet<'_>, version: i64) -> bool {
    usize::try_from(version - BASELINE_VERSION)
        .ok()
        .and_then(|index| set.migrations.get(index))
        .is_some_and(|migration| migration.rebuilds_tables)
}

/// Refuse a migration that needs foreign keys off when they are on (the guard did not take): nothing is changed.
fn require_foreign_keys_off(tx: &Transaction<'_>) -> Result<()> {
    let on: i64 = tx.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    if on != 0 {
        return Err(crate::error::Error::Other(anyhow!(
            "foreign keys are on for a migration that drops tables: it is refused, and the catalog is unchanged"
        )));
    }
    Ok(())
}

/// [`expected_for`] for the given schema, made on an empty database: the `Sql` steps make the layout, and the `Rust` steps
/// find no rows to convert.
pub(crate) fn expected_for(set: &SchemaSet<'_>, version: i64) -> Result<Schema> {
    let conn = Connection::open_in_memory()?;
    prepare_like_a_catalog(&conn)?;
    bring_to(&conn, set, pg_platform::native::PATH_RULES, version, &Hooks::none())?;
    Ok(Schema::read(&conn, SEEDS)?)
}

/// The [`expected_for`] schema of the current version, built once per process (every connection's `Catalog::open` checks
/// its file against it).
pub(crate) fn current_expected() -> Result<&'static Schema> {
    Ok(CURRENT.schema()?)
}

/// The pragmas `prepare` gives an empty file before its schema is made: incremental auto-vacuum (a page-1 setting, so
/// before the first table), then the connection's settings. The expected layouts are made the same way, so they describe
/// a catalog as this build makes one.
fn prepare_like_a_catalog(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    conn.execute_batch(SETTINGS)
}

/// This build's layout at its current version, as `pg_dbfile::Expected` makes it.
fn current_layout(conn: &Connection) -> rusqlite::Result<()> {
    prepare_like_a_catalog(conn)?;
    let version = current_version().map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
    bring_to(conn, &SCHEMA, pg_platform::native::PATH_RULES, version, &Hooks::none())
        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))
}

/// The current schema, made once per process.
static CURRENT: Expected = Expected::new(current_layout, SEEDS);

/// Why the file at `conn` is not this build's catalog at `version`, given `want` (its [`expected_for`] schema): `None` when
/// it is. It reads only. A file that is not a database is not a catalog, and neither is one whose `application_id` is
/// another's.
pub(crate) fn mismatch(conn: &Connection, version: i64, want: &Schema) -> Result<Option<String>> {
    let found = match Schema::read(conn, SEEDS) {
        Ok(found) => found,
        Err(e) if Damage::of(&e) == Some(Damage::NotADatabase) => return Ok(Some("not a database".to_string())),
        Err(e) => return Err(e.into()),
    };
    if let Some(difference) = found.difference(want) {
        return Ok(Some(difference.to_string()));
    }
    if version > 0 {
        let id = pg_dbfile::application_id(conn)?;
        if id != APPLICATION_ID {
            return Ok(Some(format!("application id {id}, not a catalog's")));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::library;
    use crate::browse::fixtures::under_both_rules;
    use crate::test_support::changed_since;
    use pg_dbfile::Difference;
    use pg_model::ids::FileId;
    use pg_model::rating::Rating;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    fn version(conn: &Connection) -> i64 {
        pg_dbfile::user_version(conn).unwrap()
    }

    /// A rebuild copies every table of the current schema but the transient ones, so a table a migration adds has to be
    /// listed in `CARRIED` or `TRANSIENT`, and in one only (or the rebuild would leave its rows behind without a word).
    #[test]
    fn carried_is_every_table_of_the_current_schema() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        let mut tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let (carried, transient) = (crate::rebuild::CARRIED, crate::rebuild::TRANSIENT);
        assert!(carried.iter().all(|t| !transient.contains(t)), "carried and transient are disjoint");
        let mut both: Vec<String> = carried.iter().chain(transient).map(|t| t.to_string()).collect();
        both.sort();
        tables.sort();
        assert_eq!(both, tables);
    }

    /// Every trigger of the current schema, with the test that covers it (`module::name`; an `under_both_rules!` test is
    /// named by its base name). [`every_trigger_names_the_test_that_covers_it`] fails on a trigger this list does not
    /// name, and on a name the schema does not have.
    const TRIGGER_TESTS: &[(&str, &str)] = &[
        ("files_changed", "core::pending_follows_what_the_analysis_was_made_from"),
        ("files_source_changed", "writers::a_size_change_alone_moves_the_file_in_the_thumbnail_source"),
        ("analysis_changed", "core::pending_follows_what_the_analysis_was_made_from"),
        ("analysis_removed", "writers::borders_stamp_the_file_as_changed_and_not_its_thumbnails"),
        ("ratings_stamp_insert", "schema::each_user_visible_change_names_its_file"),
        ("ratings_stamp_update", "schema::each_stamp_trigger_names_its_file_on_delete_and_update"),
        ("ratings_stamp_delete", "schema::each_stamp_trigger_names_its_file_on_delete_and_update"),
        ("border_marks_stamp_insert", "schema::each_user_visible_change_names_its_file"),
        ("border_marks_stamp_delete", "schema::each_stamp_trigger_names_its_file_on_delete_and_update"),
        ("file_writes_stamp_insert", "schema::each_user_visible_change_names_its_file"),
        ("file_writes_stamp_update", "schema::each_stamp_trigger_names_its_file_on_delete_and_update"),
        ("file_writes_stamp_delete", "schema::each_stamp_trigger_names_its_file_on_delete_and_update"),
        ("placeholders_stamp_insert", "schema::a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails"),
        ("placeholders_stamp_update", "schema::a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails"),
        ("placeholders_stamp_delete", "schema::a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails"),
        ("analysis_cut_stamp", "writers::borders_stamp_the_file_as_changed_and_not_its_thumbnails"),
        ("analysis_removed_stamp", "writers::borders_stamp_the_file_as_changed_and_not_its_thumbnails"),
        ("border_sides_stamp_insert", "writers::borders_stamp_the_file_as_changed_and_not_its_thumbnails"),
        ("border_sides_stamp_delete", "writers::borders_stamp_the_file_as_changed_and_not_its_thumbnails"),
        ("notices_uid", "inbox::events_past_the_bound_go_oldest_first_and_never_the_one_just_posted"),
        ("file_failures_stamp", "ledger::a_ledger_is_one_entry_new_again_with_a_new_failure"),
        ("files_failures_stale", "ledger::a_failure_is_one_row_for_the_file_as_it_failed"),
        ("files_failures_hidden", "ledger::the_hidden_trigger_fires_only_when_pending_or_missing_changes"),
        ("analysis_import", "ledger::a_failed_analysis_that_leaves_its_file_pending_is_hidden"),
        (
            "analysis_import_rewritten",
            "ledger::a_turn_written_back_moves_the_import_rows_rotation_and_keeps_its_serial",
        ),
        ("analysis_import_removed", "ledger::the_imports_ledger_mirrors_its_failed_analyses"),
        ("folder_failures_stamp", "roots::a_new_folder_failure_is_stamped_past_the_older_ones"),
        ("analysis_pairing_added", "writers::the_pairing_epoch_moves_with_what_the_pairs_are_made_from"),
        ("analysis_pairing_removed", "writers::the_pairing_epoch_moves_with_what_the_pairs_are_made_from"),
        ("analysis_pairing_changed", "writers::the_pairing_epoch_moves_with_what_the_pairs_are_made_from"),
    ];

    /// The schema's triggers are exactly the ones [`TRIGGER_TESTS`] names: a trigger added without a row there fails here.
    #[test]
    fn every_trigger_names_the_test_that_covers_it() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        let mut live = triggers(&conn);
        let mut named: Vec<String> = TRIGGER_TESTS.iter().map(|(trigger, _)| trigger.to_string()).collect();
        live.sort();
        named.sort();
        assert_eq!(named, live, "TRIGGER_TESTS names each trigger of the schema, once");
    }

    /// A migration's steps for the tests of the machinery: SQL, a conversion that records its source, and a SQL step that
    /// fails.
    const EXTRA: &[Step] = &[Step::Sql("CREATE TABLE extra(x INTEGER);")];
    const CONVERT: &[Step] = &[Step::Rust(a_conversion), Step::Sql("CREATE TABLE converted(x INTEGER);")];
    const NOT_SQL: &[Step] = &[Step::Sql("INSERT INTO extra VALUES (8);"), Step::Sql("NOT SQL")];

    /// The conversion of [`CONVERT`]: it is given the database's own rows, as a migration is ([`Source::Main`]).
    fn a_conversion(tx: &Transaction<'_>, source: Source, _rules: PathRules) -> Result<()> {
        assert_eq!(source, Source::Main);
        tx.execute("INSERT INTO extra VALUES (0)", [])?;
        Ok(())
    }

    /// A migration that drops nothing with dependants.
    const fn plain(steps: &'static [Step]) -> Migration {
        Migration { steps, rebuilds_tables: false }
    }

    /// A migration that drops tables with dependants: it runs with foreign keys off ([`Migration::rebuilds_tables`]).
    const fn rebuilds(steps: &'static [Step]) -> Migration {
        Migration { steps, rebuilds_tables: true }
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    fn migrations_apply_once_in_order_and_a_failed_one_rolls_back(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        let none = SchemaSet { baseline: BASELINE, migrations: &[] };
        bring_to(&conn, &none, rules, BASELINE_VERSION, &Hooks::none()).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations, the second a rebuild with a conversion: both run, in order, once.
        let later = [plain(EXTRA), rebuilds(CONVERT)];
        let set = SchemaSet { baseline: BASELINE, migrations: &later };
        bring_to(&conn, &set, rules, BASELINE_VERSION + 2, &Hooks::none()).unwrap();
        bring_to(&conn, &set, rules, BASELINE_VERSION + 2, &Hooks::none()).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(count(&conn, "extra"), 1, "the conversion ran once");
        assert_eq!(count(&conn, "converted"), 0);
        // A failed migration leaves the catalog at the last good version, none of its steps made.
        let broken = [plain(EXTRA), rebuilds(CONVERT), plain(NOT_SQL)];
        let broken_set = SchemaSet { baseline: BASELINE, migrations: &broken };
        let fresh = Connection::open_in_memory().unwrap();
        assert!(bring_to(&fresh, &broken_set, rules, BASELINE_VERSION + 3, &Hooks::none()).is_err());
        assert_eq!(version(&fresh), BASELINE_VERSION + 2);
        assert_eq!(count(&fresh, "extra"), 1);
        assert_eq!(count(&fresh, "converted"), 0);
    }

    under_both_rules!(
        migrations_apply_once_in_order_and_a_failed_one_rolls_back,
        migrations_apply_once_in_order_and_a_failed_one_rolls_back_windows,
        migrations_apply_once_in_order_and_a_failed_one_rolls_back_posix
    );

    fn expected_schemas_follow_the_version(rules: PathRules) {
        let later = [plain(EXTRA)];
        let set = SchemaSet { baseline: BASELINE, migrations: &later };
        let at_one = expected_for(&set, BASELINE_VERSION).unwrap();
        let at_two = expected_for(&set, BASELINE_VERSION + 1).unwrap();
        assert!(matches!(at_one.difference(&at_two), Some(Difference::Missing { .. })));
        assert!(matches!(at_two.difference(&at_one), Some(Difference::Unexpected { .. })));
        assert!(expected_for(&set, 0).unwrap().is_empty());
        // A database at a version matches the expected schema of that version.
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &set, rules, BASELINE_VERSION, &Hooks::none()).unwrap();
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(&at_one), None);
    }

    under_both_rules!(
        expected_schemas_follow_the_version,
        expected_schemas_follow_the_version_windows,
        expected_schemas_follow_the_version_posix
    );

    /// The baseline makes the schema this build expects at its version, and that schema is the one a file is checked
    /// against: a fresh catalog has no difference from it.
    #[test]
    fn a_fresh_catalog_is_the_expected_schema() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        assert_eq!(version(&conn), current_version().unwrap());
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(current_expected().unwrap()), None);
    }

    /// A fresh catalog with two files (ids 1 and 2) in one folder, for the tests of the layout's own rules.
    fn fresh_with_two_files() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, x'61'), (2, 1, 2, 1, x'62', 'jpg', 'image', 1, 1, x'62');",
        )
        .unwrap();
        conn
    }

    /// `files`, `folders` and `roots` are AUTOINCREMENT: a deleted row's id is never given to a later one, so the next id
    /// is one past the highest ever made, not one past the highest kept.
    #[test]
    fn a_deleted_row_id_is_never_reissued() {
        let conn = fresh_with_two_files();
        conn.execute("DELETE FROM files WHERE id = 2", []).unwrap();
        conn.execute(
            "INSERT INTO files(volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
             VALUES (1, 3, 1, x'63', 'jpg', 'image', 1, 1, x'63')",
            [],
        )
        .unwrap();
        let file: i64 = conn.query_row("SELECT max(id) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(file, 3, "file 2 is not reissued");

        conn.execute("INSERT INTO roots(path, added) VALUES (x'01', 0), (x'02', 0)", []).unwrap();
        conn.execute("DELETE FROM roots WHERE id = 2", []).unwrap();
        conn.execute("INSERT INTO roots(path, added) VALUES (x'03', 0)", []).unwrap();
        let root: i64 = conn.query_row("SELECT max(id) FROM roots", [], |r| r.get(0)).unwrap();
        assert_eq!(root, 3, "root 2 is not reissued");

        conn.execute("INSERT INTO folders(path, key) VALUES (x'02', x'02')", []).unwrap();
        conn.execute("DELETE FROM folders WHERE id = 2", []).unwrap();
        conn.execute("INSERT INTO folders(path, key) VALUES (x'03', x'03')", []).unwrap();
        let folder: i64 = conn.query_row("SELECT max(id) FROM folders", [], |r| r.get(0)).unwrap();
        assert_eq!(folder, 3, "folder 2 is not reissued");
    }

    /// `hashes_owed` is a queue of analyses: its row goes with its analysis, whether the analysis is deleted or its file is.
    #[test]
    fn a_queued_hash_goes_with_its_analysis() {
        let conn = fresh_with_two_files();
        for file in [1, 2] {
            conn.execute(
                "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed) VALUES (?1, 1, 1, 1, 0, 'image', 0)",
                [file],
            )
            .unwrap();
            conn.execute("INSERT INTO hashes_owed(file_id) VALUES (?1)", [file]).unwrap();
        }
        conn.execute("DELETE FROM analysis WHERE file_id = 1", []).unwrap();
        assert_eq!(count(&conn, "hashes_owed"), 1, "the other file's row stays");
        conn.execute("DELETE FROM files WHERE id = 2", []).unwrap();
        assert_eq!(count(&conn, "hashes_owed"), 0, "the file's analysis, and its row, went with the file");
    }

    /// `present_pdq_pairs` lists the pairs of files that are both present: a pair with a missing file is left out.
    #[test]
    fn a_pair_with_a_missing_file_is_not_present() {
        let conn = fresh_with_two_files();
        conn.execute("INSERT INTO pdq_pairs(a, b, distance) VALUES (1, 2, 5)", []).unwrap();
        assert_eq!(count(&conn, "present_pdq_pairs"), 1);
        conn.execute("UPDATE files SET missing = 1 WHERE id = 2", []).unwrap();
        assert_eq!(count(&conn, "present_pdq_pairs"), 0);
    }

    /// A queued write's value is within its kind's range (the CHECK), and so is its temp's value.
    #[test]
    fn a_queued_write_is_in_its_kinds_range() {
        let conn = fresh_with_two_files();
        conn.execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (1, 'rating', 5, 0)", []).unwrap();
        conn.execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (2, 'rotation', 3, 0)", []).unwrap();
        assert!(
            conn.execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (2, 'rating', 6, 0)", [])
                .is_err(),
            "a rating is 0 to 5"
        );
        assert!(
            conn.execute("INSERT INTO file_writes(file_id, kind, value, queued) VALUES (1, 'rotation', 4, 0)", [])
                .is_err(),
            "a rotation is 0 to 3"
        );
        assert!(
            conn.execute(
                "INSERT INTO file_writes(file_id, kind, value, queued, temp_value) VALUES (2, 'rating', 3, 0, 6)",
                []
            )
            .is_err(),
            "and the temp's value"
        );
        assert_eq!(count(&conn, "file_writes"), 2);
    }

    /// A new catalog made by `prepare` is the baseline at version 1, with incremental auto-vacuum and no VACUUM owed.
    #[test]
    fn a_new_catalog_is_the_baseline_with_no_vacuum_owed() {
        let dir = pg_testkit::run_folder("scratch_schema", "fresh-catalog").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = crate::Catalog::prepare(&path, crate::test_support::versions(), PathRules::Windows).unwrap();
        assert_eq!(version(&c.conn), BASELINE_VERSION);
        let mode: i64 = c.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2, "incremental");
        let owed: i64 = c.conn.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).unwrap();
        assert_eq!(owed, 0);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog whose file has no incremental auto-vacuum is owed a VACUUM (its pragma came too late to take effect, or
    /// was never set); one made with it is not. The mark is the one the next start acts on.
    #[test]
    fn a_catalog_without_incremental_auto_vacuum_is_owed_a_vacuum() {
        let owed = |conn: &Connection| -> i64 {
            conn.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).unwrap()
        };
        let plain = Connection::open_in_memory().unwrap();
        bring_to(&plain, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        assert_eq!(owed(&plain), 1);

        let incremental = Connection::open_in_memory().unwrap();
        incremental.pragma_update(None, "auto_vacuum", "INCREMENTAL").unwrap();
        bring_to(&incremental, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none())
            .unwrap();
        assert_eq!(owed(&incremental), 0);
    }

    /// The stamps of a rating, a border mark and a queued write are triggers of their own on delete and on update as
    /// well as on insert: taking each back (or changing it again) names its file, as the insert did (N-13).
    #[test]
    fn each_stamp_trigger_names_its_file_on_delete_and_update() {
        let c = library(PathRules::Windows);
        c.set_rating(FileId::new(1), Rating::new(3), false).unwrap();
        c.set_border_mark(FileId::new(2), true).unwrap();
        c.queue_rotation(FileId::new(3), Turns::new(1)).unwrap();
        let (_, now) = changed_since(&c, 0);

        // The rating is changed (its row updated), then taken away (its row deleted), then the border mark, then the
        // queued turn is changed again and written.
        c.set_rating(FileId::new(1), Rating::new(4), false).unwrap();
        let (changed, now) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(1)], "a rating's update");
        c.set_rating(FileId::new(1), None, false).unwrap();
        let (changed, now) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(1)], "a rating's delete");
        c.set_border_mark(FileId::new(2), false).unwrap();
        let (changed, now) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(2)], "a border mark's delete");
        c.queue_rotation(FileId::new(3), Turns::new(2)).unwrap();
        let (changed, now) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(3)], "a queued write's update");
        // Written: the turn is taken off the row, which a zero deletes (`write_done`).
        let job = crate::queue::Job {
            file_id: FileId::new(3),
            write: pg_model::write::Write::Turns(Turns::new(3)),
            temp: None,
            attempts: 0,
        };
        c.write_done(&job, crate::queue::WriteEffect::Nothing).unwrap();
        let (changed, _) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(3)], "a queued write's delete");
    }

    /// Every user-visible change stamps its file (`files.changed`), in the same statement as the change: the file
    /// itself, its analysis, its rating, its border mark and its queued write. A view is not a file's change.
    #[test]
    fn each_user_visible_change_names_its_file() {
        let c = library(PathRules::Windows);
        let (all, now) = changed_since(&c, 0);
        assert!(all.len() >= 4, "every file the library made is a change: {all:?}");
        // Nothing has changed since `now`.
        assert_eq!(changed_since(&c, now), (vec![], now));

        c.set_rating(FileId::new(1), Rating::new(3), false).unwrap();
        c.set_border_mark(FileId::new(2), true).unwrap();
        c.queue_rotation(FileId::new(3), Turns::new(1)).unwrap();
        let (changed, after) = changed_since(&c, now);
        assert_eq!(changed, [FileId::new(1), FileId::new(2), FileId::new(3)]);
        assert!(after > now);
        // What was seen at `after` is not reported again.
        assert_eq!(changed_since(&c, after), (vec![], after));

        // A view is not a file's change.
        c.save_view(&pg_model::view::SavedView {
            name: "v".into(),
            listing: pg_model::view::Listing {
                folder: pg_model::rules::Merged::from(vec![pg_model::name::Name::from("p")]),
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(changed_since(&c, after), (vec![], after));
    }

    /// No trigger on a `files` insert: a new row's `pending`, `seq` and `changed` are set by the insert itself
    /// (`writers::a_scans_new_files_are_pending_and_unpaired_under_one_change_number`).
    #[test]
    fn no_trigger_on_files_inserts() {
        let c = library(PathRules::Windows);
        let inserts: i64 = c
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'files' AND sql LIKE '%AFTER INSERT%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inserts, 0);
    }

    /// The placeholder stamps its file as changed, and not its thumbnails: stored, replaced and removed, each one stamps
    /// the file, and only that.
    fn a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, x'61');",
        )
        .unwrap();
        let state = || -> (i64, i64) {
            conn.query_row("SELECT changed, seq FROM files WHERE id = 1", [], |r| {
                Ok((r.get(0).unwrap(), r.get(1).unwrap()))
            })
            .unwrap()
        };
        let meta_seq =
            || -> i64 { conn.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |r| r.get(0)).unwrap() };
        let (changed, seq) = state();
        conn.execute(
            "INSERT INTO placeholders(file_id, size, mtime, rotation, version, hash) VALUES (1, 1, 1, 0, 1, x'0102')",
            [],
        )
        .unwrap();
        let (stored, thumbs) = state();
        assert_eq!((stored, thumbs), (meta_seq(), seq), "stored: the file is changed, its thumbnails are not");
        assert!(stored > changed);
        conn.execute("UPDATE placeholders SET hash = x'0304' WHERE file_id = 1", []).unwrap();
        let (replaced, thumbs) = state();
        assert_eq!((replaced, thumbs), (meta_seq(), seq), "replaced: the same");
        assert!(replaced > stored);
        conn.execute("DELETE FROM placeholders WHERE file_id = 1", []).unwrap();
        let (removed, thumbs) = state();
        assert_eq!((removed, thumbs), (meta_seq(), seq), "removed: the same");
        assert!(removed > replaced);
    }

    under_both_rules!(
        a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails,
        a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails_windows,
        a_placeholder_stamps_its_file_as_changed_and_not_its_thumbnails_posix
    );

    fn triggers(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'trigger' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// The stored text of each macro the schema's SQL shares with the runtime statements contains the macro's expansion,
    /// whitespace-normalised: a change to a macro that the baseline keeps as text is seen here.
    #[test]
    fn the_macros_in_the_stored_text_are_the_ones_the_code_uses() {
        fn normalized(text: &str) -> String {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        let stored = normalized(BASELINE);
        let expansions = [
            ("stale!", stale!()),
            ("next_seq!", next_seq!()),
            ("seq!", seq!()),
            ("live_failures!", live_failures!()),
            ("kind_video!", pg_model::kind_video!()),
            ("write_rating!", pg_model::write_rating!()),
            ("write_rotation!", pg_model::write_rotation!()),
        ];
        for (name, text) in expansions {
            assert!(stored.contains(&normalized(text)), "{name} expands to {text:?}, not in the stored text");
        }
        // `kept_events!()` is a number in the notices trigger's LIMIT, not a fragment: asserted in that context.
        assert!(stored.contains(&format!("WHERE key IS NULL) - {}))", kept_events!())), "kept_events!() context");
        // `pairing_version!()` is an integer: its seed row is the stored text.
        assert!(stored.contains(&format!("'pairing_version', {})", pg_model::pairing_version!())));
    }
}
