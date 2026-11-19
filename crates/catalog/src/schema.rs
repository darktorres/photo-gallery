//! The catalog's schema: the SQL that makes a new catalog, the version a file is at, and how a file is brought to a
//! version. A file is compared with [`expected`], the schema this build makes in memory from the same SQL, so the
//! layout checked is the layout created.
//!
//! ## Storage
//!
//! The inbox's and the ledgers' layout is in `crates/catalog/src/schema.rs` (`TO_3_NOTICES`, `TO_4_FAILURES`,
//! `TO_8_NOTIFY`). Notable:
//!
//! - `ledger_ack.dismissed_to` (a serial, not a time);
//! - `notices.rev`, the index `notices(kind, root)`, and `notices_uid` recreated with the Events' bound;
//! - `file_failures.hidden`, the partial index `file_failures_live`, `files_failures_stale` (not on import rows, and not on `missing`), and the triggers `files_failures_hidden` and `analysis_import_*`;
//! - `folder_failures` with its serial stamp, and `scan_leases`, which a rebuild doesn't carry (`rebuild::TRANSIENT`).
//!
//! A migration is a list of steps (`schema::Step`): SQL that changes the layout, or a Rust conversion of rows
//! (`crate::convert`), which finds the old shape by the columns it has, never by the version, and is idempotent. The
//! rebuild's copy runs every `Rust` step of every migration on the copied rows, so a catalog rebuilt from any version is
//! converted as its migrations would have converted it.

use anyhow::{Context, anyhow};
use pg_dbfile::{Damage, Expected, Schema};
use pg_model::rules::PathRules;
use rusqlite::{Connection, Transaction};

use crate::autoincrement::ids_autoincrement;
use crate::convert::{Source, convert_notify, convert_session, pairs_owed_and_hashes, retire_kinds};
use crate::error::Result;
use crate::transaction::write_transaction;

/// One step of a migration: SQL that changes the layout, or a conversion of rows (`crate::convert`), which reads the
/// old shape from its [`Source`] and is shaped-guarded and idempotent. Every data conversion is a `Rust` step, so the
/// rebuild's copy runs the same code (`rebuild::copy_tables` runs every `Rust` step with `Source::Carried`). `PathRules`:
/// the rules the catalog's paths are stored under.
#[derive(Clone, Copy)]
pub(crate) enum Step {
    Sql(&'static str),
    Rust(fn(&Transaction<'_>, Source, PathRules) -> Result<()>),
}

/// Migrations (`PRAGMA user_version` counts them). A new catalog gets [`BASELINE`] at version [`BASELINE_VERSION`],
/// then every migration in turn; `MIGRATIONS[i]` takes version `BASELINE_VERSION + i` to the next, its steps in order,
/// in one transaction with the version.
///
/// The baseline is frozen: since the first migration (version 2), every schema change is a migration, which converts
/// the rows whose meaning changed instead of copying them as they are, and the baseline takes comment edits only (a
/// catalog at version 1 is compared with it). A catalog whose schema differs from this build's at its version is
/// rebuilt when it is prepared (`Catalog::prepare`): a new catalog is made from the baseline and the
/// migrations and takes the old rows with their ids (the rebuild's `carry_rows`, which runs the migrations' `Rust`
/// steps on them), and the old file is moved aside. A catalog of this build's schema at an earlier version is migrated
/// forward, keeping its rows.
pub(crate) const BASELINE_VERSION: i64 = 1;
/// One migration: the steps that take a catalog one version on, in order, in one transaction with the version.
/// `rebuilds_tables`: the migration drops tables that have dependants. `DROP TABLE` of a parent runs an implicit `DELETE`
/// with foreign keys on, which cascades into its children (their analyses, ratings, queued writes), so such a migration
/// runs with foreign keys off and `legacy_alter_table` on (see [`ForeignKeysOff`]), and checks that they are off before its
/// first step. The pragma is a no-op inside a transaction, so it is set around the migration, not in it.
pub(crate) struct Migration {
    pub(crate) steps: &'static [Step],
    pub(crate) rebuilds_tables: bool,
}

/// A baseline and the migrations after it: the layout a catalog has at each version. [`SCHEMA`] is this build's.
#[derive(Clone, Copy)]
pub(crate) struct SchemaSet<'a> {
    pub(crate) baseline: &'a str,
    pub(crate) migrations: &'a [Migration],
}

/// This build's schema: [`BASELINE`] and [`MIGRATIONS`].
pub(crate) const SCHEMA: SchemaSet<'static> = SchemaSet { baseline: BASELINE, migrations: MIGRATIONS };

/// What a caller watches while a catalog is brought to a version. Nothing yet: the fault points and the progress reports
/// come in Phase 3, through this same parameter, so the signature of [`bring_to`] changes once.
pub(crate) struct Hooks;

impl Hooks {
    pub(crate) fn none() -> Self {
        Self
    }
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration { steps: &[Step::Sql(TO_2_CHANGE_STAMPS)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_3_NOTICES)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_4_FAILURES)], rebuilds_tables: false },
    Migration { steps: TO_5_SESSION, rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_6_THUMB_VIEWS)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_7_WRITE_CHECKS)], rebuilds_tables: false },
    Migration { steps: TO_8_NOTIFY, rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_9_CHEAPER_WRITES)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_10_LEDGER_NOT_WRITTEN)], rebuilds_tables: false },
    Migration { steps: TO_11_NOTIFY_FIXES, rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_12_PAIRING_PASS)], rebuilds_tables: false },
    Migration { steps: TO_13_PAIRS_OWED, rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_14_PAIRING_EPOCH)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_15_HASHES_OWED)], rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_16_NEW_FILES)], rebuilds_tables: false },
    Migration { steps: TO_17_PLACEHOLDERS, rebuilds_tables: false },
    Migration { steps: &[Step::Sql(TO_18_ANALYSIS_TOUCHES)], rebuilds_tables: false },
    // Version 18 to 19 drops tables that have dependants: its one flagged migration (see `Migration::rebuilds_tables`).
    Migration { steps: &[Step::Sql(TO_19_ROOT_REMOVALS), Step::Rust(ids_autoincrement)], rebuilds_tables: true },
    Migration { steps: &[Step::Sql(TO_20_DROP_THUMB_FOLDERS)], rebuilds_tables: false },
];

/// The version [`TO_8_NOTIFY`] takes a catalog from: the tests' "as that build left one".
#[cfg(test)]
pub(crate) const BEFORE_NOTIFY: i64 = 7;

/// Version 2: `files.changed` (what a client that kept its list fetches again by) is stamped for what the grid shows of a file's borders,
/// which the baseline's triggers missed. The borders pass writes the cut into the analysis after the import, and sets
/// its sides (`Catalog::store_borders`); a pairing's replacement deletes them, and the analysis with them. `files.seq`
/// is not touched: the thumbnails are not made from the borders.
const TO_2_CHANGE_STAMPS: &str = concat!(
    stamp!(
        "analysis_cut_stamp",
        "UPDATE OF cut_left, cut_top, cut_right, cut_bottom",
        "analysis",
        new,
        when "old.cut_left IS NOT new.cut_left OR old.cut_top IS NOT new.cut_top OR old.cut_right IS NOT new.cut_right\n    OR old.cut_bottom IS NOT new.cut_bottom"
    ),
    stamp!("analysis_removed_stamp", "DELETE", "analysis", old),
    stamp!("border_sides_stamp_insert", "INSERT", "border_sides", new),
    stamp!("border_sides_stamp_delete", "DELETE", "border_sides", old),
);

/// Version 3: the inbox (see `inbox.rs`), what the app told the user, kept until it is dismissed.
///
/// A row is a `pg_model::notice::Notice` as JSON (`data`), with what the inbox sorts and filters by beside it. A kept
/// state (`key`, its kind's code, folder and part) is one row that the next post of its key replaces and counts; an event
/// has no key, and each post is a row. `root` is the library folder it is about: removing the folder removes its rows
/// (the folders' ids are reused, so a row must not outlive its folder). `uid` is `meta.seq` when the row was made: with
/// the id (reused too) it names one row, so a mark or a dismissal the window sends for a row since gone does nothing.
/// Times are Unix seconds, as `scans`'.
const TO_3_NOTICES: &str = concat!(
    "CREATE TABLE notices(\n",
    "    id        INTEGER PRIMARY KEY,\n",
    "    uid       INTEGER NOT NULL DEFAULT 0,\n",
    "    key       TEXT UNIQUE,\n",
    "    part      TEXT,\n",
    "    root      INTEGER REFERENCES roots(id) ON DELETE CASCADE,\n",
    "    level     INTEGER NOT NULL,   -- 0 info, 1 warning, 2 error\n",
    "    source    TEXT NOT NULL,      -- Source::code\n",
    "    kind      TEXT NOT NULL,      -- Notice::code\n",
    "    data      TEXT NOT NULL,      -- the notice, JSON\n",
    "    count     INTEGER NOT NULL DEFAULT 1,\n",
    "    created   INTEGER NOT NULL,\n",
    "    updated   INTEGER NOT NULL,\n",
    "    read      INTEGER,\n",
    "    dismissed INTEGER\n",
    ");\n",
    "CREATE INDEX notices_live ON notices(dismissed, updated DESC);\n",
    "CREATE INDEX notices_root ON notices(root);\n",
    "CREATE TRIGGER notices_uid AFTER INSERT ON notices BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE notices SET uid = ",
    seq!(),
    " WHERE id = new.id;\nEND;\n",
);

/// Version 4: the failure ledger (`crate::ledger`): the files a pass couldn't make its result of, one row per file and
/// pass, for the state of the file it failed on (`mtime`, `size`, `rotation`). A change to that state, or the file going
/// missing, removes the row (`files_failures_stale`): the next pass tries the file as it is now. A pass's failure ledger
/// is one inbox entry, read from these rows; `ledger_ack` is what the user has seen of each entry (`seen_to`: the newest
/// row they were shown, by `serial`, which is `meta.seq` when the row was made) and whether they read or dismissed it.
/// `retry`: the pass tries the file again (1), or not until it changes, a retry or a new `THUMBS_VERSION` (0: it
/// panicked). `thumbs_version` is the `pg_render::THUMBS_VERSION` the thumbnails' rows were recorded under
/// (`set_thumbs_version`).
const TO_4_FAILURES: &str = concat!(
    "CREATE TABLE file_failures(\n",
    "    id         INTEGER PRIMARY KEY,\n",
    "    file_id    INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,\n",
    "    pass       TEXT NOT NULL,      -- FilePass::code: 'thumbs' | 'borders'\n",
    "    mtime      INTEGER NOT NULL,   -- the file as it failed\n",
    "    size       INTEGER NOT NULL,\n",
    "    rotation   INTEGER NOT NULL,\n",
    "    error      TEXT NOT NULL,\n",
    "    retry      INTEGER NOT NULL,\n",
    "    serial     INTEGER NOT NULL DEFAULT 0,\n",
    "    first_seen INTEGER NOT NULL,   -- Unix seconds\n",
    "    UNIQUE(file_id, pass)\n",
    ");\n",
    "CREATE INDEX file_failures_serial ON file_failures(pass, serial);\n",
    "CREATE TABLE ledger_ack(\n",
    "    kind      TEXT PRIMARY KEY,   -- FilePass::code: 'thumbs' | 'borders' | 'import'\n",
    "    seen_to   INTEGER NOT NULL,   -- a serial (the import's: files.seq)\n",
    "    read      INTEGER,\n",
    "    dismissed INTEGER\n",
    ");\n",
    "CREATE TRIGGER file_failures_stamp AFTER INSERT ON file_failures BEGIN\n    ",
    next_seq!(),
    ";\n    UPDATE file_failures SET serial = ",
    seq!(),
    " WHERE id = new.id;\nEND;\n",
    "CREATE TRIGGER files_failures_stale AFTER UPDATE OF mtime, size, rotation, missing ON files\n",
    "WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation\n",
    "    OR old.missing IS NOT new.missing\n",
    "BEGIN\n    DELETE FROM file_failures WHERE file_id = new.id;\nEND;\n",
    "INSERT INTO meta(key, value) VALUES ('thumbs_version', 0);\n",
);

/// Version 5: the session is a listing (`pg_model::view::Listing`) in one row, with the saved views' columns, which the
/// views' row code reads and writes (`listing::listing_at`, `listing_values`). It was a store of text keys; the row is
/// made from them once, by `convert::convert_session`, between the new table's creation and the old one's drop. The
/// steps' SQL is the text version 5 was first made with, so the layout it leaves is the one catalogs already have.
const TO_5_SESSION: &[Step] = &[Step::Sql(TO_5_SESSION_ROW), Step::Rust(convert_session), Step::Sql(TO_5_SESSION_SWAP)];

const TO_5_SESSION_ROW: &str = r"CREATE TABLE session_row(
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    folder    BLOB NOT NULL,     -- as views.folder
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);
";

const TO_5_SESSION_SWAP: &str = "DROP TABLE session;
ALTER TABLE session_row RENAME TO session;
";

/// Version 6: the thumbnail cache reads the catalog through its views alone. `thumb_sources` decides which files can
/// have a thumbnail (present, and analyzed without an error for the file as it is now), and gives a video's poster
/// time resolved (0 where the analysis has none; NULL for an image), so the cache's query only compares the rows with
/// its own entries. It is still one table with lookups by key, which SQLite flattens into the pass's query (the
/// `files_seq` index is kept). `thumb_folders` is the largest folder id (-1 for none): it sizes the pass's work list,
/// and is a view of its own, since `thumb_clock` is the change number the pass takes its snapshot at.
const TO_6_THUMB_VIEWS: &str = concat!(
    "DROP VIEW thumb_sources;
CREATE VIEW thumb_sources AS
    SELECT f.id, f.volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.mtime, f.rotation, f.size, f.seq,
           CASE WHEN f.kind = '",
    pg_model::kind_video!(),
    "' THEN coalesce((SELECT poster_ms FROM analysis WHERE file_id = f.id), 0)
           END AS poster_ms
    FROM files f
    WHERE f.missing = 0
      AND EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id AND error IS NULL AND mtime = f.mtime);
CREATE VIEW thumb_folders AS SELECT coalesce(max(id), -1) AS max_id FROM folders;
"
);

/// The rule a queued write's value follows (`pg_model::write::Write::from_stored`): a rating's stars 0 to 5, or 0 to 3
/// quarter turns, in the value and in the temp's.
macro_rules! write_value_ok {
    () => {
        concat!(
            "CASE kind WHEN '",
            pg_model::write_rating!(),
            "' THEN value BETWEEN 0 AND 5 AND coalesce(temp_value BETWEEN 0 AND 5, 1)
                ELSE value BETWEEN 0 AND 3 AND coalesce(temp_value BETWEEN 0 AND 3, 1) END"
        )
    };
}

/// Version 7: the write queue's value is tied to its kind by a `CHECK`, as `ratings.rating` is: the rule the rows are
/// read with (`write_value_ok!`). SQLite adds no `CHECK` in place, so the table is made again with it and takes the
/// rows that keep it: one that breaks it was never a valid write, and is dropped (the write is lost). The index and
/// the stamps go with the old table, and are made again as they were.
const TO_7_WRITE_CHECKS: &str = concat!(
    "CREATE TABLE file_writes_checked(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('",
    pg_model::write_rating!(),
    "', '",
    pg_model::write_rotation!(),
    "')),
    value    INTEGER NOT NULL,  -- write::Write::stored: a rating's stars, or quarter turns
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       BLOB,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind),
    CHECK (",
    write_value_ok!(),
    ")
);
INSERT INTO file_writes_checked(file_id, kind, value, queued, attempts, error, temp, temp_value)
    SELECT file_id, kind, value, queued, attempts, error, temp, temp_value FROM file_writes WHERE ",
    write_value_ok!(),
    ";
DROP TABLE file_writes;
ALTER TABLE file_writes_checked RENAME TO file_writes;
CREATE INDEX file_writes_next ON file_writes((temp IS NULL), queued);
",
    stamp!("file_writes_stamp_insert", "INSERT", "file_writes", new),
    stamp!("file_writes_stamp_update", "UPDATE", "file_writes", new),
    stamp!("file_writes_stamp_delete", "DELETE", "file_writes", old),
);

/// Version 8: the inbox's storage as the docs below describe it. The layout is added first, the rows are
/// converted (`convert::convert_notify`), and the column they were converted from is dropped.
const TO_8_NOTIFY: &[Step] = &[Step::Sql(TO_8_NOTIFY_ADD), Step::Rust(convert_notify), Step::Sql(TO_8_NOTIFY_DROP)];

/// - **A ledger's dismissal is a serial** (`ledger_ack.dismissed_to`): the entry is shown while it has live rows and
///   none was dismissed, or one came after the dismissal. Reading it later no longer hides it.
/// - **A notice's revision** (`notices.rev`): it rises when a state's data changes, and a mark names the revision it
///   was made for, so a mark sent for one never lands on the next.
/// - **Events are bounded by `notices_uid`**: past `kept_events!()` keyless rows, each insert deletes the oldest by
///   insertion (`uid`), read ones first, never the row inserted and never a state (bounded by its keys).
/// - **`notices(kind, root)`**: what a clear reads to find whether it has anything to remove.
/// - **The import's ledger is rows** (`pass = 'import'`), kept by the triggers on `analysis`: they mirror the failed
///   analysis, with the state of the bytes it read, so a re-analysis of the same bytes keeps its serial and a changed
///   file that fails again is new. `files_failures_stale` no longer touches them, nor any row when a file goes missing.
/// - **`file_failures.hidden`**: the file is missing (or, for the import, pending), kept by `files_failures_hidden`; the
///   inbox reads the rest through the partial index `file_failures_live`.
/// - **The folders a walk couldn't read** (`folder_failures`): one row per folder, by its exact path, tied to the library
///   folder that holds it; `walk` is the `scans` row of the walk that last saw it, which a later complete walk of a folder
///   above it sweeps by (`ScanLease::mark_missing`). `reason`: `unreadable` | `mount-refused` | `refused`.
/// - **The scan leases** (`scan_leases`, transient: a rebuild carries none): one row per walk running (`id` its `scans`
///   row), so two walks of overlapping folders never interleave (`Catalog::begin_scan`).
const TO_8_NOTIFY_ADD: &str = concat!(
    "ALTER TABLE ledger_ack ADD COLUMN dismissed_to INTEGER;
ALTER TABLE notices ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
CREATE INDEX notices_kind_root ON notices(kind, root);
DROP TRIGGER notices_uid;
CREATE TRIGGER notices_uid AFTER INSERT ON notices BEGIN
    ",
    next_seq!(),
    ";
    UPDATE notices SET uid = ",
    seq!(),
    " WHERE id = new.id;
    DELETE FROM notices WHERE new.key IS NULL AND id IN (
        SELECT id FROM notices WHERE key IS NULL AND id <> new.id ORDER BY read IS NULL, uid
        LIMIT max(0, (SELECT count(*) FROM notices WHERE key IS NULL) - ",
    kept_events!(),
    "));
END;
ALTER TABLE file_failures ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
CREATE INDEX file_failures_live ON file_failures(pass, serial) WHERE ",
    live_failures!(),
    ";
DROP TRIGGER files_failures_stale;
CREATE TRIGGER files_failures_stale AFTER UPDATE OF mtime, size, rotation ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation
BEGIN
    DELETE FROM file_failures WHERE file_id = new.id AND pass <> 'import';
END;
CREATE TRIGGER files_failures_hidden AFTER UPDATE OF pending, missing ON files
WHEN old.pending IS NOT new.pending OR old.missing IS NOT new.missing
BEGIN
    UPDATE file_failures SET hidden = (new.missing OR (pass = 'import' AND new.pending))
    WHERE file_id = new.id AND hidden IS NOT (new.missing OR (pass = 'import' AND new.pending));
END;
CREATE TRIGGER analysis_import_failed AFTER INSERT ON analysis WHEN new.error IS NOT NULL BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed, f.missing
        FROM files f WHERE f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error
        WHERE file_failures.error IS NOT excluded.error;
END;
CREATE TRIGGER analysis_import_fixed AFTER INSERT ON analysis WHEN new.error IS NULL BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import';
END;
CREATE TRIGGER analysis_import_rewritten AFTER UPDATE OF size, mtime, rotation ON analysis WHEN new.error IS NOT NULL
BEGIN
    UPDATE file_failures SET size = new.size, mtime = new.mtime, rotation = new.rotation
    WHERE file_id = new.file_id AND pass = 'import';
END;
CREATE TRIGGER analysis_import_removed AFTER DELETE ON analysis BEGIN
    DELETE FROM file_failures WHERE file_id = old.file_id AND pass = 'import';
END;
CREATE TABLE folder_failures(
    id         INTEGER PRIMARY KEY,
    root       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path       BLOB NOT NULL UNIQUE,   -- exact bytes, as folders.path
    reason     TEXT NOT NULL,          -- 'unreadable' | 'mount-refused' | 'refused'
    error      TEXT NOT NULL,
    walk       INTEGER NOT NULL,       -- the scans row of the walk that last saw it
    serial     INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL        -- Unix seconds
);
CREATE INDEX folder_failures_serial ON folder_failures(serial);
CREATE INDEX folder_failures_root ON folder_failures(root);
CREATE TRIGGER folder_failures_stamp AFTER INSERT ON folder_failures BEGIN
    ",
    next_seq!(),
    ";
    UPDATE folder_failures SET serial = ",
    seq!(),
    " WHERE id = new.id;
END;
CREATE TABLE scan_leases(
    id        INTEGER PRIMARY KEY,     -- the walk's scans row
    root      BLOB NOT NULL,           -- the folder it walks, as scans.root
    holder    TEXT NOT NULL,           -- who walks it, for the refusal's words
    heartbeat INTEGER NOT NULL         -- Unix seconds: renewed with each batch the walk writes
);
"
);

/// [`TO_8_NOTIFY`]'s last step: the acknowledgement's old dismissal time, converted.
const TO_8_NOTIFY_DROP: &str = "ALTER TABLE ledger_ack DROP COLUMN dismissed;\n";

/// Version 9: what version 8 made a write cost, measured (`just perf --budgets`), brought back to what the work needs.
/// - **`files_fresh`**: the pairing's fresh rows (`pair::fresh_rows`, unpaired with an identity, by the scan that saw
///   them), in id order, so each batch of them is a seek from the last one's id. Read through the root's folders, every
///   batch scanned all of the root's rows again: quadratic in a first scan's rows.
/// - **`analysis_import`**: the import ledger's one trigger on a new analysis, in place of `analysis_import_failed` and
///   `analysis_import_fixed`. Its `WHEN` is cheap for the common case, a file analysed without an error that has no
///   ledger row, which then writes nothing; a delete run for every analysis that succeeded cost the import a fifth of
///   its batch. A failure still writes its row, as before: the row is removed when the bytes changed, and kept (with
///   the new error) when they didn't.
const TO_9_CHEAPER_WRITES: &str =
    "CREATE INDEX files_fresh ON files(last_scan) WHERE unpaired = 1 AND volume IS NOT NULL;
DROP TRIGGER analysis_import_failed;
DROP TRIGGER analysis_import_fixed;
CREATE TRIGGER analysis_import AFTER INSERT ON analysis
WHEN new.error IS NOT NULL OR EXISTS (SELECT 1 FROM file_failures WHERE file_id = new.file_id AND pass = 'import')
BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (new.error IS NULL OR mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed, f.missing
        FROM files f WHERE new.error IS NOT NULL AND f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error
        WHERE file_failures.error IS NOT excluded.error;
END;
";

/// Version 10: a `ledger-not-written` notice is the thumbnails' only (the borders' and the import's ledger rows are
/// written in the transaction of their own work, so a failure there fails the batch). The rows an earlier build kept
/// for the other passes (`part` is the pass) have nothing that clears them now, so they go.
const TO_10_LEDGER_NOT_WRITTEN: &str = "DELETE FROM notices WHERE kind = 'ledger-not-written' AND part <> 'thumbs';";

/// Version 11: two fixes that a review of the notifications found.
/// - **The import row's `hidden`**: `analysis_import` (`TO_9_CHEAPER_WRITES`) set it from `files.missing` alone, so a
///   failed analysis that left its file pending (the file's size differs from the analysis's) made a row the grid's failed
///   filter hid while the inbox listed it. It is `missing OR pending`, as `files_failures_hidden` keeps it, on an insert and
///   on a conflict. A `pending` that changes after the insert is `files_failures_hidden`'s, and the row is there by then.
/// - **The retired kinds** (`Notice::RETIRED`) go, and every ledger row's `hidden` is recomputed (`convert::retire_kinds`).
const TO_11_NOTIFY_FIXES: &[Step] = &[Step::Sql(TO_11_IMPORT_HIDDEN), Step::Rust(retire_kinds)];

/// Version 12: the pairs are found by a pass after the import (`pg_import::pair`), not by the import. An analysis that
/// owes its pairs is `dups_state = 'due'`: the import sets it for every picture with orientation hashes and a reliable
/// quality, and the pass sets 'done' once the pairs are written. Every existing analysis was paired inline under this
/// build's constants, so the column takes 'done' and the catalog keeps its pairs.
/// - **`pdq_pairs.variant` goes**: nothing reads it, and the stored side's orientation is not the relative transform of
///   the pair, so it could not be read as one. The pairs are `(a, b, distance)`.
/// - **`meta.pairing_version`** is seeded with this build's `PAIRING_VERSION`, which the join's output depends on:
///   `Catalog::prepare` marks every eligible analysis due when it changes (`lifecycle::set_pairing_version`).
const TO_12_PAIRING_PASS: &str = concat!(
    "ALTER TABLE analysis ADD COLUMN dups_state TEXT NOT NULL DEFAULT 'done' CHECK (dups_state IN ('done', 'due'));
CREATE INDEX analysis_dups_due ON analysis(file_id) WHERE dups_state = 'due';
DROP VIEW present_pdq_pairs;
ALTER TABLE pdq_pairs DROP COLUMN variant;
CREATE VIEW present_pdq_pairs AS
    SELECT p.* FROM pdq_pairs p JOIN files fa ON fa.id = p.a JOIN files fb ON fb.id = p.b
    WHERE fa.missing = 0 AND fb.missing = 0;
INSERT INTO meta(key, value) VALUES ('pairing_version', ",
    pg_model::pairing_version!(),
    ");
"
);

/// Version 13: what the pairing pass owes is a queue, and the stored hashes are the four kept orientations.
/// - **`pairs_owed`**, one row per analysis that owes its pairs, replaces `analysis.dups_state` and its partial index.
///   Marking a picture done is a delete from a narrow table, not an update of the wide `analysis` row: the pass's
///   187,804 writes made 98.7 MB of WAL as updates and 3.4 MB as deletes, 4.2x faster. The row is keyed by the analysis
///   (`ON DELETE CASCADE`), so an analysis that goes (a re-analysis's REPLACE, a path taken over by another file) takes
///   it, and no row outlives the analysis it is owed for.
/// - **`pdq_dihedral` keeps orientations 0..4** (128 bytes, `pg_model::pdq::ORIENTATIONS`): 4..8 are their transposes bit
///   for bit, which the join makes. `pdq` goes: it was the blob's first 32 bytes, and the pairs' guards compare the
///   blob. The data step (`convert::pairs_owed_and_hashes`) checks every blob before it cuts it, and carries what was owed.
/// - **`meta.vacuum_owed`**: dropping the columns leaves their pages in use until a `VACUUM`, which can't run in the
///   migration's transaction; `Catalog::prepare` runs it after the commit (`lifecycle::vacuum_if_owed`).
/// - **`meta.held_exts`**: the set of extensions of the last build that opened or scanned the catalog
///   (`pg_model::media::held_exts`). `Catalog::prepare` looks for files of extensions it doesn't hold only when the set
///   is another (`lifecycle::mark_unheld_missing`), instead of reading every file at every start. Seeded empty, so the
///   first start looks.
const TO_13_PAIRS_OWED: &[Step] =
    &[Step::Sql(TO_13_QUEUE), Step::Rust(pairs_owed_and_hashes), Step::Sql(TO_13_DROP_COLUMNS)];

const TO_13_QUEUE: &str = "CREATE TABLE pairs_owed(
    file_id INTEGER PRIMARY KEY REFERENCES analysis(file_id) ON DELETE CASCADE
);
INSERT INTO meta(key, value) VALUES ('vacuum_owed', 1);
INSERT INTO meta(key, value) VALUES ('held_exts', '');
";

const TO_13_DROP_COLUMNS: &str = "DROP INDEX analysis_dups_due;
ALTER TABLE analysis DROP COLUMN dups_state;
ALTER TABLE analysis DROP COLUMN pdq;
";

/// Version 14: **`meta.pairing_epoch`**, a count that moves on whenever an analysis the pairing pass's writes are
/// guarded by may have changed, so a write that finds it where the pass's load read it needs no guard
/// (`PairsWriter::write`): the guard reads every owner's and partner's analysis, which was over half the write
/// (docs/reference/PROFILING.md, "The write"). A picture passes the guard while its analysis is there with the hashes
/// and quality the load read, so the count moves:
/// - on every insert into `analysis`, whatever the row: a REPLACE's own delete fires no delete trigger (recursive
///   triggers are off), so a new analysis over an old one is seen only here;
/// - on a delete from `analysis` of a row with hashes: one without was no picture of any load. A file's delete reaches
///   here through its cascade;
/// - on an update that changes a row's `file_id`, `pdq_dihedral` or `pdq_quality`.
///
/// Owed rows move nothing: the guard never read them. A row owed again since the load (`owe_pairs_again`, which also
/// drops every pair) is marked done by a write that made its picture's pairs from the analysis it still holds, guarded
/// or not, and the pictures the pass does not mark done stay owed. Another build's pairing is `meta.pairing_version`,
/// which every write checks. A trigger on `pairs_owed` would also move the count once per row of `owe_pairs_again`,
/// doubling its 137 ms on 187,804 pictures.
///
/// The count only grows, so a write that finds another keeps guarding to the pass's end.
const TO_14_PAIRING_EPOCH: &str = concat!(
    "INSERT INTO meta(key, value) VALUES ('pairing_epoch', 0);
CREATE TRIGGER analysis_pairing_added AFTER INSERT ON analysis BEGIN
    ",
    next_pairing_epoch!(),
    ";
END;
CREATE TRIGGER analysis_pairing_removed AFTER DELETE ON analysis WHEN old.pdq_dihedral IS NOT NULL BEGIN
    ",
    next_pairing_epoch!(),
    ";
END;
CREATE TRIGGER analysis_pairing_changed AFTER UPDATE OF file_id, pdq_dihedral, pdq_quality ON analysis
WHEN old.file_id IS NOT new.file_id OR old.pdq_dihedral IS NOT new.pdq_dihedral OR old.pdq_quality IS NOT new.pdq_quality
BEGIN
    ",
    next_pairing_epoch!(),
    ";
END;
"
);

/// Version 15: **`hashes_owed`**, one row per analysis that the import stored without its duplicate hashes (duplicates
/// detection was off: `Hashing::Skip`). The Hashes pass (`pg_import::hashes`) makes them, and each row goes with its
/// analysis (`ON DELETE CASCADE`), so a re-analysis or a failure takes it. Shaped like `pairs_owed` (version 13): a narrow
/// queue, not a flag on the wide `analysis` row. Every analysis that exists was made with its hashes, so it starts empty.
const TO_15_HASHES_OWED: &str = "CREATE TABLE hashes_owed(
    file_id INTEGER PRIMARY KEY REFERENCES analysis(file_id) ON DELETE CASCADE
);
";

/// [`TO_11_NOTIFY_FIXES`]' trigger: the import's ledger row is made hidden when its file is missing or pending, and a
/// conflict updates `hidden` as well as `error`.
/// Version 16: **`files_added` goes.** Whoever inserts a `files` row sets its `pending`, `seq` and `changed` itself: the
/// scan's writer does (`ScanWriter::record_file`, one change number per batch) and so does the fixture. The trigger
/// made each insert a multi-write statement with a statement journal, for three columns the insert can set itself: the
/// first scan of 200,000 files spent 42% of its instructions there (docs/temp/scan-optimizations-validation.md).
const TO_16_NEW_FILES: &str = "DROP TRIGGER files_added;
DROP INDEX files_fresh;
ALTER TABLE files DROP COLUMN last_scan;
";

/// Version 17: **`placeholders`**, the ThumbHash of each file (`pg_thumbs::placeholders`), kept in its own table, keyed to
/// the file and carrying the identity it was made at (size, mtime, catalog rotation) and the version of its definition
/// (`pg_model::analysis::PLACEHOLDER_VERSION`). A file owes one exactly when no row matches its identity and version, so
/// staleness is the row's own: an analysis's REPLACE (a re-import) cannot wipe a placeholder, a write is a narrow insert
/// (not an update of the wide `analysis` row: migration 13's 98.7 MB against 3.4 MB), and nothing is queued.
///
/// The import no longer makes ThumbHashes, so `analysis.thumbhash` goes, and the existing ones move here (the data step,
/// `convert::placeholders_from_thumbhashes`, which runs on the old shape in a migration and on the carried rows in a
/// rebuild). Their `placeholders` rows are stamped `changed` like the borders are, and a turn or a size change does not
/// move them (the placeholder is shown until the pass replaces it). `analysis.width`/`height` are stamped with the cut:
/// the borders pass writes the decode's own size, so a corrected size is a change a client shows.
const TO_17_PLACEHOLDERS: &[Step] = &[
    Step::Sql(TO_17_CREATE),
    Step::Rust(crate::convert::placeholders_from_thumbhashes),
    Step::Sql(TO_17_DROP_THUMBHASH),
];

const TO_17_CREATE: &str = "CREATE TABLE placeholders(
    file_id  INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    size     INTEGER NOT NULL,
    mtime    INTEGER NOT NULL,
    rotation INTEGER NOT NULL,
    version  INTEGER NOT NULL,
    hash     BLOB NOT NULL
);
";

/// The `analysis.thumbhash` column goes (after its data moved), and the stamps are made: the placeholders' own, and the
/// cut's, which now names the width and height too. The triggers are made after the data step, so that step's inserts
/// stamp nothing.
const TO_17_DROP_THUMBHASH: &str = concat!(
    "ALTER TABLE analysis DROP COLUMN thumbhash;
DROP TRIGGER analysis_cut_stamp;
",
    stamp!("placeholders_stamp_insert", "INSERT", "placeholders", new),
    stamp!(
        "placeholders_stamp_update",
        "UPDATE OF hash, size, mtime, rotation, version",
        "placeholders",
        new,
        when "old.hash IS NOT new.hash OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.version IS NOT new.version"
    ),
    stamp!("placeholders_stamp_delete", "DELETE", "placeholders", old),
    stamp!(
        "analysis_cut_stamp",
        "UPDATE OF cut_left, cut_top, cut_right, cut_bottom, width, height",
        "analysis",
        new,
        when "old.cut_left IS NOT new.cut_left OR old.cut_top IS NOT new.cut_top OR old.cut_right IS NOT new.cut_right
    OR old.cut_bottom IS NOT new.cut_bottom OR old.width IS NOT new.width OR old.height IS NOT new.height"
    ),
);

/// Version 18: **`analysis_added` goes.** An analysis's insert no longer updates its file's `pending`, `seq` and
/// `changed` in a trigger. The writers that insert an analysis make that update themselves, after the insert, with the
/// same `meta.seq` bump and the same values the trigger set (`AnalysisWriter::touch_file`; `writers::touch_analysis`
/// for the tests and fixtures). The trigger made each insert a multi-write statement with a statement journal, as
/// `files_added` did (version 16). Every writer of an analysis row is one of those, and the fixtures call
/// `touch_analysis` too.
const TO_18_ANALYSIS_TOUCHES: &str = "DROP TRIGGER analysis_added;\n";

/// Version 19: **`root_removals`**, the marker of a folder being removed from the library: from the moment it leaves the
/// library until its files, their thumbnails and its folders are gone (`crate::removal`). `path` is the folder as the
/// rules join it, the same bytes as `roots.path`, and overlaps are tested with the rules, not with `=`. `deferred` counts
/// the files kept because an unfinished write is recorded for them. The ids of `files`, `folders` and `roots` are then
/// never reused (`ids_autoincrement`), so a removed id names nothing.
/// Version 20: `thumb_folders` (version 6's view of the largest folder id) goes. The work lists are sized by what they
/// hold, not by the largest folder id, which after removals is far above the folders there are, and a folder id is never
/// reused (AUTOINCREMENT, version 19), so that bound only grows. Its own version, since a catalog at 19 already has it.
const TO_20_DROP_THUMB_FOLDERS: &str = "DROP VIEW thumb_folders;\n";

const TO_19_ROOT_REMOVALS: &str = "CREATE TABLE root_removals(
    id       INTEGER PRIMARY KEY,
    path     BLOB NOT NULL UNIQUE,
    started  INTEGER NOT NULL,   -- Unix seconds
    deferred INTEGER NOT NULL DEFAULT 0
);
";

const TO_11_IMPORT_HIDDEN: &str = "DROP TRIGGER analysis_import;
CREATE TRIGGER analysis_import AFTER INSERT ON analysis
WHEN new.error IS NOT NULL OR EXISTS (SELECT 1 FROM file_failures WHERE file_id = new.file_id AND pass = 'import')
BEGIN
    DELETE FROM file_failures WHERE file_id = new.file_id AND pass = 'import'
        AND (new.error IS NULL OR mtime IS NOT new.mtime OR size IS NOT new.size OR rotation IS NOT new.rotation);
    INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
        SELECT new.file_id, 'import', new.mtime, new.size, new.rotation, new.error, 0, new.analyzed,
               (f.missing OR f.pending)
        FROM files f WHERE new.error IS NOT NULL AND f.id = new.file_id
        ON CONFLICT(file_id, pass) DO UPDATE SET error = excluded.error, hidden = excluded.hidden
        WHERE file_failures.error IS NOT excluded.error OR file_failures.hidden IS NOT excluded.hidden;
END;
";

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
    let migrations = i64::try_from(MIGRATIONS.len()).context("too many migrations")?;
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
    _hooks: &Hooks,
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
            for step in migration.steps {
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

/// The schema a catalog at `version` has, as this build makes it in memory: the baseline and the migrations up to
/// `version`. Version 0 (no catalog) is the empty database.
pub(crate) fn expected(version: i64) -> Result<Schema> {
    expected_for(&SCHEMA, version)
}

/// [`expected`] for the given schema, made on an empty database: the `Sql` steps make the layout, and the `Rust` steps
/// find no rows to convert.
pub(crate) fn expected_for(set: &SchemaSet<'_>, version: i64) -> Result<Schema> {
    let conn = Connection::open_in_memory()?;
    bring_to(&conn, set, pg_platform::native::PATH_RULES, version, &Hooks::none())?;
    Ok(Schema::read(&conn, SEEDS)?)
}

/// The [`expected`] schema of the current version, built once per process (every connection's `Catalog::open` checks
/// its file against it).
pub(crate) fn current_expected() -> Result<&'static Schema> {
    Ok(CURRENT.schema()?)
}

/// This build's layout at its current version, as `pg_dbfile::Expected` makes it.
fn current_layout(conn: &Connection) -> rusqlite::Result<()> {
    let version = current_version().map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
    bring_to(conn, &SCHEMA, pg_platform::native::PATH_RULES, version, &Hooks::none())
        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))
}

/// The current schema, made once per process.
static CURRENT: Expected = Expected::new(current_layout, SEEDS);

/// Why the file at `conn` is not this build's catalog at `version`, given `want` (its [`expected`] schema): `None` when
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

pub(crate) const BASELINE: &str = concat!(
    r"
CREATE TABLE scans(
    id       INTEGER PRIMARY KEY,
    root     BLOB NOT NULL,      -- the folder as stored (Name bytes, PathRules::join)
    started  INTEGER NOT NULL,   -- Unix seconds
    finished INTEGER
);
CREATE TABLE folders(
    id   INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,   -- absolute, no trailing separator (PathRules::join)
    key  BLOB NOT NULL           -- the folder's key (PathRules::folder_key): the lookup by folder
);
CREATE TABLE files(
    id        INTEGER PRIMARY KEY,
    -- The identity: the volume's 64-bit serial (Windows) or device number (Unix), and the file ID. NULL for a
    -- displaced row (its file is another one now, or it is gone): it keeps its id, folder, name, ratings and content,
    -- and waits to be paired (`unpaired`). Two NULLs are not equal, so the UNIQUE holds for any number of them.
    volume    INTEGER,
    file_ref  INTEGER,           -- NTFS file ID; the low 48 bits are the MFT record number
    -- The creation time (FILETIME ticks), where the file system gives one; NULL where it doesn't (drvfs). Two
    -- values agree when either is NULL or they are equal: see the scanner's identity rules.
    created   INTEGER,
    folder_id INTEGER NOT NULL REFERENCES folders(id),
    name      BLOB NOT NULL,     -- the file name as the OS has it (Name bytes)
    ext       TEXT NOT NULL,     -- the name's last extension, lower case (media::Ext)
    kind      TEXT NOT NULL,     -- media::Kind::key, from the extension: 'image' | 'video'
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,  -- FILETIME: 100 ns since 1601 UTC
    last_scan INTEGER NOT NULL REFERENCES scans(id),
    missing   INTEGER NOT NULL DEFAULT 0,
    rotation  INTEGER NOT NULL DEFAULT 0,  -- quarter turns clockwise kept here, for files that can't be rotated
                                           -- in place (`pg_edit::rotate`); applied after EXIF orientation
    order_key BLOB NOT NULL,     -- where it sorts by name, all drives merged (PathRules::order_key)
    pending   INTEGER NOT NULL DEFAULT 1,  -- the import has to analyze it (again): kept by the triggers below
    seq       INTEGER NOT NULL DEFAULT 0,  -- meta.seq when its thumbnails' source last changed: kept by triggers
    changed   INTEGER NOT NULL DEFAULT 0,  -- meta.seq at its last user-visible change (a client's resync): kept by triggers
    -- Not yet resolved by the scanner's pairing: a row made for a file it has not seen before, or a displaced row. The
    -- pairing runs at the end of every scan of its root and clears the flag from the rows it leaves as new files.
    unpaired  INTEGER NOT NULL DEFAULT 0,
    UNIQUE(volume, file_ref)
);
CREATE INDEX files_folder ON files(folder_id, name);
-- A file's identity from its file ID alone: the pairing's candidates on another volume (tier 2).
CREATE INDEX files_ref ON files(file_ref);
CREATE INDEX folders_key ON folders(key);
-- The grid's name order and every folder's files (a range of keys: browse::read::scope).
CREATE INDEX files_order ON files(order_key) WHERE missing = 0;
-- The folder tree's counts, from the index alone.
CREATE INDEX files_present ON files(folder_id) WHERE missing = 0;
-- The import's work in on-disk order (Catalog::pending_import): a round with nothing to import is one probe.
CREATE INDEX files_to_import ON files(volume, file_ref & 0xFFFFFFFFFFFF) WHERE missing = 0 AND pending = 1;
-- The files changed since the thumbnail pass last looked (thumbs::pending_in).
CREATE INDEX files_seq ON files(seq);
-- The files changed since a change number (a client's resync).
CREATE INDEX files_changed ON files(changed);
CREATE TABLE analysis(
    file_id     INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,  -- analyze::IMPORT_VERSION
    size        INTEGER NOT NULL,  -- the file as analyzed
    mtime       INTEGER NOT NULL,
    rotation    INTEGER NOT NULL,  -- files.rotation it was decoded with
    kind        TEXT NOT NULL,     -- files.kind it was analyzed as
    analyzed    INTEGER NOT NULL,  -- Unix seconds
    error       TEXT,              -- set when the file couldn't be read or decoded; the rest is then NULL
    file_hash   BLOB,              -- tier 0a: BLAKE3 of the bytes
    pixel_hash  BLOB,              -- tier 0b: XXH3-128 of the upright decoded pixels
    format      TEXT,
    orientation INTEGER,
    width       INTEGER,           -- upright, full size
    height      INTEGER,
    pdq         BLOB,              -- tier 1: 32 bytes, Meta's byte order
    pdq_quality INTEGER,
    pdq_dihedral BLOB,             -- tier 1: the 8 orientations' hashes, 32 bytes each (the first is pdq's), for pairs
    thumbhash   BLOB,
    frames      INTEGER,           -- frames in the file: > 1 is an animated image (or a video)
    -- When it was taken, as wall-clock time (no time zone) in Unix seconds: the frame every date filter and sort
    -- compares in. A camera's own clock is taken as it is; a UTC instant (a video's creation time) is converted to
    -- the local wall clock of this system's zone (`taken_zone`), so it is also kept below, as the instant.
    taken       INTEGER,
    -- The UTC instant `taken` was converted from (a video's creation time), NULL for a camera's wall clock. `taken` is
    -- computed from it again when the system's zone changes (`set_taken_zone`), so no zone is lost at import.
    taken_utc   INTEGER,
    duration_ms INTEGER,           -- videos
    poster_ms   INTEGER,           -- videos: where the frame for the ThumbHash and the thumbnails is
    -- Borders (borders::detect) on the stored pixels, found after the import by a pass of their own: what a crop
    -- cuts from the left, top, right and bottom (detection::Detection::cut), all 0 for none. NULL until looked for,
    -- and where they aren't (videos, animated and tiny images, failures). `borders_due`: to be looked for, by this
    -- build's detector (the import sets it; Catalog::prepare too when meta.borders_version differs).
    cut_left    INTEGER,
    cut_top     INTEGER,
    cut_right   INTEGER,
    cut_bottom  INTEGER,
    borders_due INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX analysis_file_hash ON analysis(file_hash);
CREATE INDEX analysis_pixel_hash ON analysis(pixel_hash);
-- The files to retry (`gallery import --retry`) and the failure count.
CREATE INDEX analysis_failed ON analysis(file_id) WHERE error IS NOT NULL;
-- The borders to look for (Catalog::pending_borders).
CREATE INDEX analysis_borders_due ON analysis(file_id) WHERE borders_due = 1;
-- Every side borders::detect found: the accepted ones (which make the cut) and the rejected candidates.
CREATE TABLE border_sides(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass     INTEGER NOT NULL,  -- the peeling round (nested borders come later)
    edge     TEXT NOT NULL CHECK (edge IN ('top', 'bottom', 'left', 'right')),
    kind     TEXT NOT NULL CHECK (kind IN ('solid', 'blur')),
    class    TEXT NOT NULL CHECK (class IN ('black', 'white', 'color', 'blur')),
    depth    INTEGER NOT NULL,  -- stored pixels from the edge of what earlier rounds left
    color    INTEGER,           -- solid: 0xRRGGBB at the edge
    straight REAL NOT NULL,
    dev      REAL,              -- solid and measured: flatness
    drift    REAL,
    rejected TEXT,              -- detection::Reason key; NULL: accepted
    PRIMARY KEY(file_id, pass, edge)
) WITHOUT ROWID;
-- The class facet (query::BorderClass).
CREATE INDEX border_sides_class ON border_sides(class, file_id) WHERE rejected IS NULL;
-- Files whose borders the user marked as not a border (the review). Kept when the borders are looked for again.
CREATE TABLE border_marks(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    marked  INTEGER NOT NULL  -- Unix seconds
);
-- Tier 1 candidates: PDQ distance <= pg_model::pdq::MATCH_DISTANCE between reliable hashes, any of the 8 orientations.
CREATE TABLE pdq_pairs(
    a        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,  -- a < b
    b        INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    distance INTEGER NOT NULL,
    variant  INTEGER NOT NULL,  -- dihedral variant of the newer file that matched (pdq::hash_rgb_dihedral)
    PRIMARY KEY(a, b)
) WITHOUT ROWID;
CREATE INDEX pdq_pairs_b ON pdq_pairs(b);
-- Pairs whose files are both present. A missing file keeps its pairs, in case it comes back.
CREATE VIEW present_pdq_pairs AS
    SELECT p.* FROM pdq_pairs p JOIN files fa ON fa.id = p.a JOIN files fb ON fb.id = p.b
    WHERE fa.missing = 0 AND fb.missing = 0;
-- What the thumbnail cache's pass reads (its own file, so it knows the catalog only through this): the files with an
-- analysis, and their folder's path. Version 6 remakes it (TO_6_THUMB_VIEWS). One table and no join, with the analysis and the folder as lookups by key: SQLite
-- flattens it into the pass's query only then (two joins never flatten together), which keeps the files_seq index.
CREATE VIEW thumb_sources AS
    SELECT f.id, f.volume, f.file_ref, f.folder_id,
           (SELECT path FROM folders WHERE id = f.folder_id) AS folder,
           f.name, f.kind, f.mtime, f.rotation, f.size, f.missing, f.seq,
           (SELECT error FROM analysis WHERE file_id = f.id) AS error,
           (SELECT mtime FROM analysis WHERE file_id = f.id) AS analyzed_mtime,
           (SELECT poster_ms FROM analysis WHERE file_id = f.id) AS poster_ms
    FROM files f WHERE EXISTS (SELECT 1 FROM analysis WHERE file_id = f.id);

-- The change number now, which the thumbnail pass takes its snapshot at: one row, the view the pass reads it through.
CREATE VIEW thumb_clock AS SELECT value AS seq FROM meta WHERE key = 'seq';

-- The library: folders the user added, which a rescan walks again. None is inside another.
CREATE TABLE roots(
    id    INTEGER PRIMARY KEY,
    path  BLOB NOT NULL UNIQUE,  -- absolute, no trailing separator (PathRules::join)
    added INTEGER NOT NULL       -- Unix seconds
);

-- Ratings: the catalog is the source of truth, for every format. Unrated files have no row.
CREATE TABLE ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    rating  INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated INTEGER NOT NULL  -- Unix seconds
);
-- The rating the file itself holds, as last read by the import or written by the app (0–5; no row: none).
-- A file rating that differs from this was changed by another program, and is adopted (store_analysis).
CREATE TABLE file_ratings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    value   INTEGER NOT NULL CHECK (value BETWEEN 0 AND 5)
);
-- Changes waiting to be written into the files (the writeback crate), so a write survives quitting. `value` is
-- the rating, or the quarter turns clockwise still to apply. `temp` is set once the copy of the complete new
-- content beside the original, carrying `temp_value`, into the original starts (the original may be part-written
-- from then); a row with it set is finished by copying it in (the file keeps its NTFS identity), and is never given
-- up. The user may change `value` meanwhile: what is left after `temp_value` is written stays queued.
-- `temp` is the temp file's path as the rules join it (the bytes, like `files`' folder), not text.
CREATE TABLE file_writes(
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
    value    INTEGER NOT NULL,
    queued   INTEGER NOT NULL,  -- Unix seconds
    attempts INTEGER NOT NULL DEFAULT 0,
    error    TEXT,
    temp       BLOB,
    temp_value INTEGER,
    PRIMARY KEY(file_id, kind)
);
-- The writer's next job (Catalog::next_write orders by these): the interrupted copies first, then the oldest, read off
-- the index in order, without sorting the whole queue for each job.
CREATE INDEX file_writes_next ON file_writes((temp IS NULL), queued);

-- Saved views (ImageRater's bookmarks): a folder, the filters and an order.
CREATE TABLE views(
    name      TEXT PRIMARY KEY,
    folder    BLOB NOT NULL,     -- the merged folder's parts, joined (PathRules::join_merged); empty = all folders
    recursive INTEGER NOT NULL,
    query     TEXT NOT NULL,     -- query::Query as JSON
    sort      TEXT NOT NULL      -- query::Sort as JSON
);
-- The session as text keys, until version 5 makes it one row (TO_5_SESSION, where it is described).
CREATE TABLE session(
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- About the catalog itself. `catalog_id` is made with it: a cache keyed by file ids (the thumbnails) checks it,
-- since a rebuilt catalog numbers its files anew. `import_version` is the IMPORT_VERSION `files.pending` was
-- computed for (Catalog::prepare recomputes it when the build's differs). `seq` is the change sequence's last number.
-- `borders_version` is the BORDERS_VERSION `analysis.borders_due` was computed for. `taken_zone` is the system zone's
-- offsets the UTC-sourced `analysis.taken` values were computed with (`set_taken_zone`).
CREATE TABLE meta(
    key   TEXT PRIMARY KEY,
    value ANY NOT NULL
);
INSERT INTO meta(key, value) VALUES
    ('catalog_id', lower(hex(randomblob(16)))), ('import_version', 0), ('seq', 0), ('borders_version', 0),
    ('taken_zone', '');

-- files.pending and files.seq, kept in the statements that change what they depend on, whoever writes them. The
-- WHEN clauses keep a rescan that changes nothing from writing anything. (Version 16 drops `files_added` and version 18
-- drops `analysis_added`: a new row's pending and seq are then set by its insert, an analysis's by its writer.)
CREATE TRIGGER files_added AFTER INSERT ON files BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.id;
END;
CREATE TRIGGER files_changed AFTER UPDATE OF size, mtime, rotation, kind ON files
WHEN old.size IS NOT new.size OR old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation
    OR old.kind IS NOT new.kind
BEGIN
    UPDATE files SET pending = ",
    stale!(),
    r" WHERE id = new.id;
END;
-- What a thumbnail is made from (thumbs::Source), and whether the file is there to show.
CREATE TRIGGER files_source_changed AFTER UPDATE OF mtime, rotation, missing, size ON files
WHEN old.mtime IS NOT new.mtime OR old.rotation IS NOT new.rotation OR old.missing IS NOT new.missing
    OR old.size IS NOT new.size
BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.id;
END;
-- A REPLACE fires only this one (recursive triggers are off): its delete half fires no trigger.
CREATE TRIGGER analysis_added AFTER INSERT ON analysis BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.file_id;
END;
-- Besides what pending depends on: whether it decoded (a thumbnail needs that) and a video's poster frame.
CREATE TRIGGER analysis_changed AFTER UPDATE OF version, size, mtime, rotation, kind, error, poster_ms ON analysis
WHEN old.version IS NOT new.version OR old.size IS NOT new.size OR old.mtime IS NOT new.mtime
    OR old.rotation IS NOT new.rotation OR old.kind IS NOT new.kind OR old.error IS NOT new.error
    OR old.poster_ms IS NOT new.poster_ms
BEGIN
    ",
    next_seq!(),
    ";
    UPDATE files SET pending = ",
    stale!(),
    ", seq = ",
    seq!(),
    ", changed = ",
    seq!(),
    r" WHERE id = new.file_id;
END;
CREATE TRIGGER analysis_removed AFTER DELETE ON analysis BEGIN
    UPDATE files SET pending = 1 WHERE id = old.file_id;
END;
",
    stamp!("ratings_stamp_insert", "INSERT", "ratings", new),
    stamp!("ratings_stamp_update", "UPDATE", "ratings", new),
    stamp!("ratings_stamp_delete", "DELETE", "ratings", old),
    stamp!("border_marks_stamp_insert", "INSERT", "border_marks", new),
    stamp!("border_marks_stamp_delete", "DELETE", "border_marks", old),
    stamp!("file_writes_stamp_insert", "INSERT", "file_writes", new),
    stamp!("file_writes_stamp_update", "UPDATE", "file_writes", new),
    stamp!("file_writes_stamp_delete", "DELETE", "file_writes", old),
);

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

    /// One migration of SQL steps.
    const EXTRA: &[Step] = &[Step::Sql("CREATE TABLE extra(x INTEGER);")];
    const SEVEN: &[Step] = &[Step::Sql("INSERT INTO extra VALUES (7);")];
    const NOT_SQL: &[Step] = &[Step::Sql("INSERT INTO extra VALUES (8);"), Step::Sql("NOT SQL")];

    /// A migration that drops nothing with dependants.
    const fn plain(steps: &'static [Step]) -> Migration {
        Migration { steps, rebuilds_tables: false }
    }

    fn migrations_apply_once_in_order_and_a_failed_one_rolls_back(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        let none = SchemaSet { baseline: BASELINE, migrations: &[] };
        bring_to(&conn, &none, rules, BASELINE_VERSION, &Hooks::none()).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION);
        // A later build adds two migrations: both run, in order, once.
        let later = [plain(EXTRA), plain(SEVEN)];
        let set = SchemaSet { baseline: BASELINE, migrations: &later };
        bring_to(&conn, &set, rules, BASELINE_VERSION + 2, &Hooks::none()).unwrap();
        bring_to(&conn, &set, rules, BASELINE_VERSION + 2, &Hooks::none()).unwrap();
        assert_eq!(version(&conn), BASELINE_VERSION + 2);
        assert_eq!(conn.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // A failed migration leaves the catalog at the last good version, none of its steps made.
        let broken = [plain(EXTRA), plain(SEVEN), plain(NOT_SQL)];
        let broken_set = SchemaSet { baseline: BASELINE, migrations: &broken };
        let fresh = Connection::open_in_memory().unwrap();
        assert!(bring_to(&fresh, &broken_set, rules, BASELINE_VERSION + 3, &Hooks::none()).is_err());
        assert_eq!(version(&fresh), BASELINE_VERSION + 2);
        assert_eq!(fresh.query_row("SELECT count(*) FROM extra", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
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

    /// Version 5 makes the session's text keys its one row: the folder's parts (one not UTF-8) joined by the rules'
    /// separator, as a view's folder is stored, and the other keys as they were. A session never saved stays empty.
    fn the_session_keys_become_its_row(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, BASELINE_VERSION + 3, &Hooks::none()).unwrap();
        conn.execute_batch(
            r#"INSERT INTO session(key, value) VALUES
                 ('folder', '["pinterest",{"b":"78ff"}]'), ('recursive', 'false'), ('sort', '{"key":"size"}');"#,
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        let row: (i64, Vec<u8>, bool, String, String) = conn
            .query_row("SELECT id, folder, recursive, query, sort FROM session", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        let folder = pg_model::rules::Merged::from(vec![
            pg_model::name::Name::from("pinterest"),
            pg_model::name::Name::from_raw(b"x\xFF"),
        ]);
        assert_eq!((row.0, row.1, row.2), (1, rules.join_merged(&folder), false));
        // The query never saved reads as the default; the sort as it was saved.
        assert_eq!(pg_query::from_json::<pg_model::query::Query>(&row.3).unwrap(), pg_model::query::Query::default());
        let sort: pg_model::query::Sort = pg_query::from_json(&row.4).unwrap();
        assert_eq!(sort, pg_model::query::Sort { key: pg_model::query::SortKey::Size, descending: false });

        let empty = Connection::open_in_memory().unwrap();
        bring_to(&empty, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        assert_eq!(empty.query_row("SELECT count(*) FROM session", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    under_both_rules!(
        the_session_keys_become_its_row,
        the_session_keys_become_its_row_windows,
        the_session_keys_become_its_row_posix
    );

    /// Version 7 keeps the queued writes whose value fits their kind and drops the others; from then on the CHECK
    /// refuses one, and the stamps still fire.
    fn a_write_out_of_its_kinds_range_is_dropped_and_refused(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, BASELINE_VERSION + 5, &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, 1, x'61'), (2, 1, 2, 1, x'62', 'jpg', 'image', 1, 1, 1, x'62');
             INSERT INTO file_writes(file_id, kind, value, queued) VALUES (1, 'rating', 4, 0), (1, 'rotation', 7, 0),
                 (2, 'rating', 9, 0), (2, 'rotation', 3, 0);",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        let kept: Vec<(i64, String, i64)> = conn
            .prepare("SELECT file_id, kind, value FROM file_writes ORDER BY file_id, kind")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(kept, [(1, "rating".to_string(), 4), (2, "rotation".to_string(), 3)]);
        assert!(conn.execute("UPDATE file_writes SET value = 6 WHERE file_id = 1", []).is_err(), "the CHECK holds");
        assert!(
            conn.execute("UPDATE file_writes SET temp_value = 4 WHERE file_id = 2", []).is_err(),
            "and for the temp's value"
        );
        let stamps = triggers(&conn);
        assert!(stamps.iter().any(|t| t == "file_writes_stamp_update"), "{stamps:?}");
    }

    under_both_rules!(
        a_write_out_of_its_kinds_range_is_dropped_and_refused,
        a_write_out_of_its_kinds_range_is_dropped_and_refused_windows,
        a_write_out_of_its_kinds_range_is_dropped_and_refused_posix
    );

    /// Version 17 moves each ThumbHash the import stored to `placeholders`, at its analysis's identity and version 1, and
    /// drops the column: the placeholder the grid shows is the one the catalog had, with no pass to make it again.
    fn version_17_moves_the_thumbhashes_to_placeholders(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, BASELINE_VERSION + 15, &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 7, 9, x'61');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                 VALUES (2, 1, 2, 1, x'62', 'jpg', 'image', 7, 9, x'62');
             INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, thumbhash)
                 VALUES (1, 2, 7, 9, 0, 'image', 0, x'0102'), (2, 2, 7, 9, 0, 'image', 0, x'');",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        let moved: Vec<(i64, i64, i64, Vec<u8>)> = conn
            .prepare("SELECT file_id, size, version, hash FROM placeholders ORDER BY file_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        // The one with a hash moves, at the analysis's size; the empty one has none to move.
        assert_eq!(moved, [(1, 7, 1, vec![1, 2])]);
        let has_column: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('analysis') WHERE name = 'thumbhash')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!has_column, "analysis.thumbhash is dropped");
    }

    under_both_rules!(
        version_17_moves_the_thumbhashes_to_placeholders,
        version_17_moves_the_thumbhashes_to_placeholders_windows,
        version_17_moves_the_thumbhashes_to_placeholders_posix
    );

    /// A placeholder is a change of its file (`files.changed`, the change a client that keeps its list fetches again by),
    /// and not of its thumbnails (`files.seq`): stored, replaced and removed, each one stamps the file, and only that.
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

    /// Version 11 makes a pending file's failed import row hidden, and drops the retired kinds, on a catalog at version 9
    /// that holds both: the row was left visible by `analysis_import`, and a `root-refused` notice by version 8.
    fn version_11_hides_the_pending_imports_and_retires_the_kinds(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, BASELINE_VERSION + 8, &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, last_scan, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, 1, x'61');
             INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
                 VALUES (1, 'import', 1, 1, 0, 'no', 0, 1, 0);
             INSERT INTO notices(level, source, kind, data, created, updated) VALUES
                 (2, 'app', 'root-refused', '{}', 0, 0), (2, 'app', 'listing-failed', '{}', 0, 0);",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        let hidden: i64 =
            conn.query_row("SELECT hidden FROM file_failures WHERE pass = 'import'", [], |r| r.get(0)).unwrap();
        assert_eq!(hidden, 1, "a pending file's import row is hidden");
        let kinds: Vec<String> = conn
            .prepare("SELECT kind FROM notices ORDER BY kind")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(kinds, ["listing-failed"]);
    }

    under_both_rules!(
        version_11_hides_the_pending_imports_and_retires_the_kinds,
        version_11_hides_the_pending_imports_and_retires_the_kinds_windows,
        version_11_hides_the_pending_imports_and_retires_the_kinds_posix
    );

    /// Version 19 makes `files`, `folders` and `roots` AUTOINCREMENT: a row deleted after the migration never has its id
    /// reissued, and the rows the migration copied keep their ids and their children. A row deleted before the migration
    /// is the accepted limit: SQLite kept no record of it, so the next id is one past the highest kept.
    fn version_19_ids_are_never_reissued_after_the_migration(rules: PathRules) {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap() - 1, &Hooks::none()).unwrap();
        conn.execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, x'00', 0);
             INSERT INTO roots(id, path, added) VALUES (1, x'00', 0);
             INSERT INTO folders(id, path, key) VALUES (1, x'01', x'01'), (2, x'02', x'02');
             INSERT INTO files(id, volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                 VALUES (1, 1, 1, 1, x'61', 'jpg', 'image', 1, 1, x'61'), (2, 1, 2, 1, x'62', 'jpg', 'image', 1, 1, x'62');
             INSERT INTO ratings(file_id, rating, updated) VALUES (2, 3, 0);",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();

        let ratings: i64 = conn.query_row("SELECT count(*) FROM ratings WHERE file_id = 2", [], |r| r.get(0)).unwrap();
        assert_eq!(ratings, 1, "the child rows of a copied file keep it");
        let fk: Vec<String> = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(fk.is_empty(), "{fk:?}");

        // Once migrated, the high-water mark is kept: deleting the top rows never lets their ids come back.
        let insert = "INSERT INTO files(volume, file_ref, folder_id, name, ext, kind, size, mtime, order_key)
                      VALUES (1, ?1, 1, ?2, 'jpg', 'image', 1, 1, ?2)";
        conn.execute(insert, rusqlite::params![3, b"c".as_slice()]).unwrap();
        conn.execute("DELETE FROM files WHERE id = 3", []).unwrap();
        conn.execute(insert, rusqlite::params![4, b"d".as_slice()]).unwrap();
        let fresh: i64 = conn.query_row("SELECT max(id) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(fresh, 4, "a deleted id (3) is not reissued");

        // The second root takes id 2; deleting it, the next root takes 3, never 2 again.
        conn.execute("INSERT INTO roots(path, added) VALUES (x'03', 0)", []).unwrap();
        conn.execute("DELETE FROM roots WHERE id = 2", []).unwrap();
        conn.execute("INSERT INTO roots(path, added) VALUES (x'04', 0)", []).unwrap();
        let root: i64 = conn.query_row("SELECT max(id) FROM roots", [], |r| r.get(0)).unwrap();
        assert_eq!(root, 3, "a deleted root id is not reissued");
    }

    under_both_rules!(
        version_19_ids_are_never_reissued_after_the_migration,
        version_19_ids_are_never_reissued_after_the_migration_windows,
        version_19_ids_are_never_reissued_after_the_migration_posix
    );

    fn triggers(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'trigger' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// The stamps of version 2 are its migration's, not the baseline's (which is frozen): a version-1 catalog taken
    /// forward by the migrations has this build's schema.
    fn the_baseline_and_the_migration_make_the_current_schema(rules: PathRules) {
        assert_eq!(current_version().unwrap(), BASELINE_VERSION + i64::try_from(MIGRATIONS.len()).unwrap());
        let stamps =
            ["analysis_cut_stamp", "analysis_removed_stamp", "border_sides_stamp_delete", "border_sides_stamp_insert"];
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, rules, BASELINE_VERSION, &Hooks::none()).unwrap();
        let before = triggers(&conn);
        assert!(stamps.iter().all(|s| !before.iter().any(|t| t == s)), "{before:?}");
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(&expected(BASELINE_VERSION).unwrap()), None);
        bring_to(&conn, &SCHEMA, rules, current_version().unwrap(), &Hooks::none()).unwrap();
        assert_eq!(version(&conn), current_version().unwrap());
        let after = triggers(&conn);
        assert!(stamps.iter().all(|s| after.iter().any(|t| t == s)), "{after:?}");
        assert!(after.iter().any(|t| t == "notices_uid"), "{after:?}");
        assert_eq!(Schema::read(&conn, SEEDS).unwrap().difference(current_expected().unwrap()), None);
    }

    under_both_rules!(
        the_baseline_and_the_migration_make_the_current_schema,
        the_baseline_and_the_migration_make_the_current_schema_windows,
        the_baseline_and_the_migration_make_the_current_schema_posix
    );

    /// The tests' version before the inbox's conversion is the one `TO_8_NOTIFY` takes a catalog from.
    #[test]
    fn before_notify_is_the_inbox_conversions_source() {
        let at = usize::try_from(BEFORE_NOTIFY - BASELINE_VERSION).unwrap();
        assert!(std::ptr::eq(MIGRATIONS[at].steps, TO_8_NOTIFY));
    }

    /// The version-10 migration drops the stored `ledger-not-written` rows of the borders and the import (a kind the
    /// build no longer posts for them), and keeps the thumbnails'.
    #[test]
    fn the_ledger_not_written_rows_of_other_passes_go() {
        let conn = Connection::open_in_memory().unwrap();
        // Version 9: the migration before TO_10 (`MIGRATIONS[8]`) is the next one to run.
        let before = BASELINE_VERSION + 8;
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, before, &Hooks::none()).unwrap();
        for (key, part) in [
            ("ledger-not-written|borders", "borders"),
            ("ledger-not-written|import", "import"),
            ("ledger-not-written|thumbs", "thumbs"),
        ] {
            conn.execute(
                "INSERT INTO notices(key, part, level, source, kind, data, created, updated)
                 VALUES (?1, ?2, 1, 'thumbs', 'ledger-not-written', '{}', 0, 0)",
                (key, part),
            )
            .unwrap();
        }
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
        let kept: Vec<String> = conn
            .prepare("SELECT part FROM notices WHERE kind = 'ledger-not-written' ORDER BY part")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(kept, ["thumbs"]);
    }

    /// Version 12 (`TO_12_PAIRING_PASS`): an older catalog's pairs are kept, and its analyses are done (they were paired
    /// inline), so nothing is owed on upgrade; the CHECK holds; `variant` is gone and `present_pdq_pairs` still reads the
    /// table; and `meta.pairing_version` is seeded with this build's.
    #[test]
    fn the_pairing_pass_keeps_the_pairs_an_older_catalog_made() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, 11, &Hooks::none()).unwrap();
        // The rows name files this test does not make: only the migration's effect on them is tested.
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(
            "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, pdq, pdq_quality, pdq_dihedral)
             VALUES (1, 1, 10, 100, 0, 'image', 0, zeroblob(32), 100, zeroblob(256));
             INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (1, 2, 5, 3);",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, 12, &Hooks::none()).unwrap();
        let state: String =
            conn.query_row("SELECT dups_state FROM analysis WHERE file_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "done");
        assert!(
            conn.execute("UPDATE analysis SET dups_state = 'skipped' WHERE file_id = 1", []).is_err(),
            "the CHECK holds"
        );
        let pairs: i64 = conn
            .query_row("SELECT count(*) FROM pdq_pairs WHERE a = 1 AND b = 2 AND distance = 5", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pairs, 1);
        let variant: i64 = conn
            .query_row("SELECT count(*) FROM pragma_table_info('pdq_pairs') WHERE name = 'variant'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(variant, 0);
        conn.query_row("SELECT count(*) FROM present_pdq_pairs", [], |r| r.get::<_, i64>(0)).unwrap();
        let seeded: i64 =
            conn.query_row("SELECT value FROM meta WHERE key = 'pairing_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(seeded, pg_model::pdq::PAIRING_VERSION);
    }

    /// Version 15 (`TO_15_HASHES_OWED`): the queue starts empty (an analysis an older catalog holds has its hashes), and a
    /// row goes with its analysis: a delete of the analysis takes it (`ON DELETE CASCADE`), as a re-analysis's REPLACE does.
    #[test]
    fn the_hashes_owed_queue_starts_empty_and_goes_with_its_analysis() {
        let conn = Connection::open_in_memory().unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, 14, &Hooks::none()).unwrap();
        // The row names a file this test does not make: only the migration's effect on it is tested.
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(
            "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, pdq_quality, pdq_dihedral)
             VALUES (1, 2, 10, 100, 0, 'image', 0, 100, zeroblob(128));",
        )
        .unwrap();
        bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, 15, &Hooks::none()).unwrap();
        let owed: i64 = conn.query_row("SELECT count(*) FROM hashes_owed", [], |r| r.get(0)).unwrap();
        assert_eq!(owed, 0);
        conn.execute_batch("PRAGMA foreign_keys = ON; INSERT INTO hashes_owed(file_id) VALUES (1);").unwrap();
        conn.execute("DELETE FROM analysis WHERE file_id = 1", []).unwrap();
        let owed: i64 = conn.query_row("SELECT count(*) FROM hashes_owed", [], |r| r.get(0)).unwrap();
        assert_eq!(owed, 0, "the row went with its analysis");
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

    /// Version 16 (`TO_16_NEW_FILES`) dropped `files_added`: no trigger on a `files` insert. A new row's `pending`, `seq`
    /// and `changed` are set by the insert itself (`writers::a_scans_new_files_are_pending_and_unpaired_under_one_change_number`).
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
}
