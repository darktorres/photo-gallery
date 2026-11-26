//! What the scan's per-file writes cost, and which get a statement journal: `stmt_journal_probe [variant]` runs one
//! variant (or, with none, each in turn) and prints, per variant, its rows, its wall time, and its connection's lookaside
//! counters. For callgrind, run one variant per process and collect its timed part only (`valgrind --tool=callgrind
//! --toggle-collect='stmt_journal_probe::timed_part' target/release/examples/stmt_journal_probe a`). `PG_PROBE_MEMSTAT=0`
//! turns SQLite's memory statistics off before any connection opens.
//!
//! Each variant makes a fresh catalog (`Catalog::prepare`, so the schema and its triggers are the app's), opens it raw
//! with the catalog's connection settings except the page cache (256 MiB here, the catalog's 8 MiB) and the automatic
//! checkpoint, which stays on here (the catalog turns it off), then adds the rows the writes need (two scans rows, 80 folders), changes the
//! schema or the settings as the variant says, then writes 4,000 files in one `BEGIN IMMEDIATE` transaction as
//! `pg_scan::write_batch` does: with `ScanWriter`'s statements prepared and held, and its `known` lookup before each
//! write. The rows are those of the scan probe's library (`folder-NN/IMG_NN_MMMM.jpg` under `/bench-lib`), in their
//! folders' merged folders as the writer makes them, with made-up identities (one interned volume), sizes and times. A
//! row seen by a walk is marked in the walk's set (`temp.scan_seen`), as the writer marks it.

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

use pg_catalog::{Catalog, Versions};
use pg_model::media::Ext;
use pg_platform::native::PATH_RULES;
use rusqlite::types::ToSql;
use rusqlite::{Connection, Statement, ffi, params};

/// The probe's result: any error, its words kept as text (a probe has no caller that matches on its errors).
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The words of a failure: `words: error` for a failed call, the words alone for a missing value.
trait Context<T> {
    fn context(self, words: &str) -> Result<T>;
}

impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, words: &str) -> Result<T> {
        self.map_err(|e| format!("{words}: {e}").into())
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, words: &str) -> Result<T> {
        self.ok_or_else(|| words.into())
    }
}

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err(format!($($arg)*).into())
    };
}

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !($cond) {
            bail!($($arg)*);
        }
    };
}

const FOLDERS: usize = 80;
const PER_FOLDER: usize = 50;
const ROOT: &str = "/bench-lib";
/// The files' volume serial; the rows hold its interned id (`Ctx::volume`).
const SERIAL: i64 = 2049;

const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY; PRAGMA busy_timeout = 5000;";

const GUARD: &str = "NOT EXISTS(SELECT 1 FROM file_writes WHERE file_id = files.id AND temp IS NOT NULL)";
/// The insert's columns: ten values (`?1`–`?10`), then `unpaired` and whatever a variant adds.
const COLUMNS: &str =
    "INSERT INTO files(volume, file_ref, created, folder_id, mid, name, ext, kind, size, mtime, unpaired";
const DO_UPDATE: &str = "ON CONFLICT(volume, file_ref) DO UPDATE SET
    folder_id = excluded.folder_id, mid = excluded.mid, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
    size = excluded.size, mtime = excluded.mtime, missing = 0, created = coalesce(files.created, excluded.created)";
/// The writer's `known` lookup, with the root's range bound as no bound (`?3`, `?4`).
const KNOWN: &str = "SELECT f.id, f.folder_id, f.name, f.ext, f.kind, f.size, f.mtime, f.missing, f.mid, f.created,
        f.unpaired, (d.path_key >= ?3 AND (?4 IS NULL OR d.path_key < ?4))
     FROM files f JOIN folders d ON d.id = f.folder_id
     WHERE f.volume = (SELECT id FROM volumes WHERE serial = ?1) AND f.file_ref = ?2";
/// The walk's seen set, as the scan makes it (`roots.rs`).
const SEEN_SET: &str = "CREATE TEMP TABLE IF NOT EXISTS scan_seen(id INTEGER PRIMARY KEY, unpaired INTEGER NOT NULL);
     CREATE INDEX IF NOT EXISTS temp.scan_seen_unpaired ON scan_seen(id) WHERE unpaired = 1;";
/// The rest of `ScanWriter`'s statements, prepared and held through the batch as the writer holds them.
const HELD: [&str; 8] = [
    "SELECT id, path, mid FROM folders WHERE path_key = ?1",
    "INSERT INTO folders(path, path_key, mid) VALUES (?1, ?2, ?3) RETURNING id",
    "UPDATE folders SET path = ?2 WHERE id = ?1",
    "SELECT id, unpaired FROM files WHERE volume = (SELECT id FROM volumes WHERE serial = ?1) AND file_ref = ?2",
    "INSERT OR IGNORE INTO temp.scan_seen(id, unpaired) VALUES (?1, ?2)",
    "UPDATE files SET created = ?2 WHERE id = ?1 AND created IS NULL",
    "UPDATE files SET volume = NULL, file_ref = NULL, missing = 1, unpaired = 1 WHERE id = ?1",
    "SELECT id FROM volumes WHERE serial = ?1",
];
/// The indexes on `files` a variant can drop (the identity's unique index can't be).
const INDEXES: [&str; 4] = ["files_mn", "files_fm", "files_to_import", "files_seq"];

#[derive(Clone)]
struct File {
    folder: usize,
    file_ref: i64,
    name: Vec<u8>,
    ext: i64,
    kind: i64,
    size: i64,
    mtime: i64,
}

struct Ctx {
    files: Vec<File>,
    /// Each folder's id and merged folder.
    folders: Vec<(i64, i64)>,
    /// The files' interned volume.
    volume: i64,
}

/// What a variant takes out of the schema before its timed part.
#[derive(Clone, Copy)]
enum Removed {
    Nothing,
    /// `files_added`.
    Added,
    /// Every trigger on `files`.
    AllOnFiles,
    /// `files_added` replaced by a trigger of only its n-th statement.
    AddedPart(usize),
    /// `files_added`, and this index.
    AddedAndIndex(&'static str),
    /// The `files_added` trigger, installed (a fresh catalog has none).
    Kept,
}

/// The `files_added` trigger's SQL, with its macros expanded. The variants that keep, split or drop it install it first,
/// so each one measures the work with the trigger in place.
const FILES_ADDED: &str = "CREATE TRIGGER files_added AFTER INSERT ON files BEGIN
    UPDATE meta SET value = value + 1 WHERE key = 'seq';
    UPDATE files SET pending = NOT EXISTS (SELECT 1 FROM analysis a WHERE a.file_id = files.id
        AND a.version >= (SELECT value FROM meta WHERE key = 'import_version')
        AND a.size = files.size AND a.mtime = files.mtime AND a.rotation = files.turns AND a.kind = files.kind),
        seq = (SELECT value FROM meta WHERE key = 'seq'), changed = (SELECT value FROM meta WHERE key = 'seq')
        WHERE id = new.id;
END;";

/// The timed part.
#[derive(Clone, Copy)]
enum Action {
    /// The app's upsert, one statement per file.
    Upsert,
    /// A plain INSERT per file; `folded` sets `files_added`'s values in it (one `meta.seq` bump for the batch).
    Plain {
        folded: bool,
    },
    /// One statement for the whole batch (`json_each`): the upsert, or the folded plain INSERT.
    SetUpsert,
    SetFolded,
    /// Every file changed (one more byte), as the upsert writes it, or as a plain UPDATE of the known row.
    ChangedUpsert,
    ChangedUpdate,
    /// Every row marked seen, one UPDATE per file, or one for the batch.
    Seen,
    SeenSet,
}

struct Variant {
    key: String,
    about: String,
    fk: bool,
    removed: Removed,
    /// The batch written by the upsert first, untimed (for the paths that need existing rows).
    prefill: bool,
    /// Lookaside slot size and count, set as the connection opens.
    lookaside: Option<(i32, i32)>,
    action: Action,
}

fn variants() -> Vec<Variant> {
    let v = |key: &str, about: &str, fk, removed, prefill, action| Variant {
        key: key.to_owned(),
        about: about.to_owned(),
        fk,
        removed,
        prefill,
        lookaside: None,
        action,
    };
    let mut all = vec![
        v("a", "upsert (the app's)", true, Removed::Nothing, false, Action::Upsert),
        v("b", "plain insert, trigger kept", true, Removed::Kept, false, Action::Plain { folded: false }),
        v("c", "plain insert, trigger folded", true, Removed::Added, false, Action::Plain { folded: true }),
        v("d", "upsert, files_added dropped", true, Removed::Added, false, Action::Upsert),
        v("f", "upsert, files_added dropped, foreign keys off", false, Removed::Added, false, Action::Upsert),
        v("g", "upsert, every files trigger dropped", true, Removed::AllOnFiles, false, Action::Upsert),
        v("h", "upsert, no files trigger, foreign keys off", false, Removed::AllOnFiles, false, Action::Upsert),
        v("t1", "upsert, files_added with its meta bump only", true, Removed::AddedPart(0), false, Action::Upsert),
        v("t2", "upsert, files_added with its files update only", true, Removed::AddedPart(1), false, Action::Upsert),
        v("k", "one upsert for the batch, trigger kept", true, Removed::Kept, false, Action::SetUpsert),
        v("k2", "one folded insert for the batch, trigger dropped", true, Removed::Added, false, Action::SetFolded),
        v("i", "changed files, upsert", true, Removed::Nothing, true, Action::ChangedUpsert),
        v("j", "changed files, plain update of the known row", true, Removed::Nothing, true, Action::ChangedUpdate),
        v("e", "seen mark (rescan)", true, Removed::Nothing, true, Action::Seen),
        v("e2", "seen mark, foreign keys off", false, Removed::Nothing, true, Action::Seen),
        v("e3", "one seen mark for the batch", true, Removed::Nothing, true, Action::SeenSet),
    ];
    for index in INDEXES {
        all.push(v(
            &format!("x-{index}"),
            &format!("as c, without {index}"),
            true,
            Removed::AddedAndIndex(index),
            false,
            Action::Plain { folded: true },
        ));
    }
    for (base, action, removed) in
        [("a", Action::Upsert, Removed::Nothing), ("c", Action::Plain { folded: true }, Removed::Added)]
    {
        for count in [200, 500, 1000] {
            let mut variant = v(
                &format!("la-{base}-{count}"),
                &format!("as {base}, lookaside 1200 x {count}"),
                true,
                removed,
                false,
                action,
            );
            variant.lookaside = Some((1200, count));
            all.push(variant);
        }
    }
    all
}

#[expect(unsafe_code, reason = "the SQLite C API's own configuration call, made before SQLite starts")]
fn main() -> Result<()> {
    if std::env::var(pg_testkit::inputs::PROBE_MEMSTAT).is_ok_and(|v| v == "0") {
        // SAFETY: called before any connection opens or SQLite initializes (the first statement of main), with the
        // option's one int argument.
        let rc = unsafe { ffi::sqlite3_config(ffi::SQLITE_CONFIG_MEMSTATUS, 0) };
        ensure!(rc == ffi::SQLITE_OK, "sqlite3_config(MEMSTATUS) returned {rc}");
    }
    let dir = pg_testkit::run_folder("scratch_catalog", "stmt-journal-probe")?;
    let files = library()?;
    let only = std::env::args().nth(1);
    let mut ran = 0;
    for (i, variant) in variants().into_iter().enumerate() {
        if only.as_ref().is_some_and(|key| *key != variant.key) {
            continue;
        }
        ran += 1;
        run(&dir.join(format!("v{i}.sqlite")), &variant, &files)?;
    }
    ensure!(ran > 0, "no variant {only:?}");
    Ok(())
}

#[expect(unsafe_code, reason = "the SQLite C API, called through the probe's own FFI declarations")]
fn run(path: &std::path::Path, variant: &Variant, files: &[File]) -> Result<()> {
    std::mem::drop(Catalog::prepare(path, Versions::current(), PATH_RULES)?);
    let conn = Connection::open(path)?;
    // The name index is on the catalog's function: every write of a file row calls it.
    pg_query::register_functions(&conn, PATH_RULES)?;
    if let Some((size, count)) = variant.lookaside {
        // SAFETY: the handle is this open connection's, used on this thread and not closed here; nothing has been
        // allocated from its lookaside yet; the buffer argument is null, so SQLite allocates the slots itself.
        let rc = unsafe {
            ffi::sqlite3_db_config(
                conn.handle(),
                ffi::SQLITE_DBCONFIG_LOOKASIDE,
                std::ptr::null_mut::<std::ffi::c_void>(),
                size,
                count,
            )
        };
        ensure!(rc == ffi::SQLITE_OK, "sqlite3_db_config(LOOKASIDE) returned {rc}");
    }
    conn.execute_batch(SETTINGS)?;
    let ctx = setup(&conn, files)?;
    if variant.prefill {
        timed_rows(&conn, &ctx, Action::Upsert)?;
    }
    remove(&conn, variant.removed)?;
    if !variant.fk {
        conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    }
    lookaside(&conn)?;
    let start = Instant::now();
    timed_part(&conn, &ctx, variant.action)?;
    let millis = start.elapsed().as_millis();
    let [hit, size, full, used] = lookaside(&conn)?;
    let rows: i64 = conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
    writeln!(
        std::io::stdout(),
        "{}\t{}\t{rows} rows\t{millis} ms\tlookaside hits {hit}, misses for size {size}, misses with slots full {full}, \
         slots used at most {used}",
        variant.key,
        variant.about
    )?;
    Ok(())
}

fn remove(conn: &Connection, removed: Removed) -> Result<()> {
    match removed {
        Removed::Nothing => {}
        Removed::Added => conn.execute_batch("DROP TRIGGER IF EXISTS files_added")?,
        Removed::AddedAndIndex(index) => {
            conn.execute_batch(&format!("DROP TRIGGER IF EXISTS files_added; DROP INDEX {index}"))?
        }
        Removed::Kept => conn.execute_batch(&format!("DROP TRIGGER IF EXISTS files_added; {FILES_ADDED}"))?,
        Removed::AllOnFiles => {
            let names: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'files'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for n in names {
                conn.execute_batch(&format!("DROP TRIGGER \"{n}\""))?;
            }
        }
        Removed::AddedPart(part) => {
            let body = FILES_ADDED
                .split_once("BEGIN")
                .and_then(|(_, rest)| rest.rsplit_once("END"))
                .map(|(body, _)| body)
                .context("files_added's body")?;
            let statements: Vec<&str> = body.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
            ensure!(statements.len() == 2, "files_added has {} statements", statements.len());
            let kept = statements.get(part).context("the statement kept")?;
            conn.execute_batch(&format!(
                "DROP TRIGGER IF EXISTS files_added; CREATE TRIGGER files_added AFTER INSERT ON files BEGIN {kept}; END;"
            ))?;
        }
    }
    Ok(())
}

/// The scan probe's library as rows: 80 folders of 50 files, in walk order.
fn library() -> Result<Vec<File>> {
    let mut files = Vec::with_capacity(FOLDERS * PER_FOLDER);
    for f in 1..=FOLDERS {
        for n in 1..=PER_FOLDER {
            let text = format!("IMG_{f:02}_{n:04}.jpg");
            let ext = Ext::of_name_bytes(text.as_bytes()).context("the extension")?;
            let index = i64::try_from(files.len())?;
            files.push(File {
                folder: f - 1,
                file_ref: 100_000 + index,
                name: text.into_bytes(),
                ext: ext.code(),
                kind: ext.kind().code(),
                size: 1_000 + index,
                mtime: 133_000_000_000_000_000 + index,
            });
        }
    }
    Ok(files)
}

/// The files' volume, the folders' rows (each in its own merged folder) and the walk's seen set, untimed.
fn setup(conn: &Connection, files: &[File]) -> Result<Ctx> {
    conn.execute_batch(SEEN_SET)?;
    let volume = conn.query_row("INSERT INTO volumes(serial) VALUES (?1) RETURNING id", [SERIAL], |r| r.get(0))?;
    let mut folders = Vec::with_capacity(FOLDERS);
    for f in 1..=FOLDERS {
        let folder = PATH_RULES.parse(&format!("{ROOT}/folder-{f:02}")).context("the folder path")?;
        let mid: i64 = conn.query_row(
            "INSERT INTO mfolders(key) VALUES (?1) RETURNING id",
            [PATH_RULES.folder_key(&folder.parts)],
            |r| r.get(0),
        )?;
        let id: i64 = conn.query_row(
            "INSERT INTO folders(path, path_key, mid) VALUES (?1, ?2, ?3) RETURNING id",
            params![PATH_RULES.join(&folder), PATH_RULES.path_key(&folder), mid],
            |r| r.get(0),
        )?;
        folders.push((id, mid));
    }
    Ok(Ctx { files: files.to_vec(), folders, volume })
}

#[inline(never)]
fn timed_part(conn: &Connection, ctx: &Ctx, action: Action) -> Result<()> {
    timed_rows(conn, ctx, action)
}

/// One batch, in one write transaction, with the writer's other statements held.
fn timed_rows(conn: &Connection, ctx: &Ctx, action: Action) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let held: Vec<Statement<'_>> = HELD.iter().map(|s| conn.prepare(s)).collect::<rusqlite::Result<_>>()?;
    match action {
        Action::Upsert => each_file(conn, ctx, &upsert_sql(), None, false)?,
        Action::Plain { folded: false } => {
            each_file(conn, ctx, &format!("{COLUMNS}) VALUES ({}, 1)", marks(10)), None, false)?
        }
        Action::Plain { folded: true } => {
            let seq = bump(conn)?;
            let sql = format!("{COLUMNS}, pending, seq, changed) VALUES ({}, 1, 1, ?11, ?11)", marks(10));
            each_file(conn, ctx, &sql, Some(seq), false)?;
        }
        Action::ChangedUpsert => each_file(conn, ctx, &upsert_sql(), None, true)?,
        Action::ChangedUpdate => each_file(
            conn,
            ctx,
            &format!(
                "UPDATE files SET folder_id = ?4, mid = ?5, name = ?6, ext = ?7, kind = ?8, size = ?9, mtime = ?10,
                    missing = 0, created = coalesce(created, ?3)
                 WHERE id = ?13 AND size IS ?11 AND mtime IS ?12 AND {GUARD}"
            ),
            None,
            true,
        )?,
        Action::SetUpsert | Action::SetFolded => {
            let folded = matches!(action, Action::SetFolded);
            known_each(conn, ctx, false)?;
            let json = batch_json(ctx)?;
            let select = "SELECT j.value->>0, j.value->>1, NULL, j.value->>2, j.value->>3, unhex(j.value->>4),
                j.value->>5, j.value->>6, j.value->>7, j.value->>8, 1";
            let n = if folded {
                let seq = bump(conn)?;
                conn.execute(
                    &format!("{COLUMNS}, pending, seq, changed) {select}, 1, ?2, ?2 FROM json_each(?1) AS j"),
                    params![json, seq],
                )?
            } else {
                conn.execute(
                    &format!(
                        "{COLUMNS}) {select} FROM json_each(?1) AS j WHERE true {DO_UPDATE}
                         WHERE files.size IS excluded.size AND {GUARD}"
                    ),
                    [json],
                )?
            };
            ensure!(n == ctx.files.len(), "{n} rows written");
        }
        Action::Seen => {
            let mut seen = conn.prepare("INSERT OR IGNORE INTO temp.scan_seen(id, unpaired) VALUES (?1, 0)")?;
            for id in ids(conn)? {
                ensure!(seen.execute([id])? == 1, "one row seen");
            }
        }
        Action::SeenSet => {
            let json = serde_json::to_string(&ids(conn)?)?;
            let n = conn.execute(
                "INSERT OR IGNORE INTO temp.scan_seen(id, unpaired) SELECT value, 0 FROM json_each(?1)",
                [json],
            )?;
            ensure!(n == ctx.files.len(), "{n} rows seen");
        }
    }
    drop(held);
    conn.execute_batch("COMMIT")?;
    Ok(())
}

fn upsert_sql() -> String {
    format!(
        "{COLUMNS}) VALUES ({}, 1) {DO_UPDATE} WHERE files.size IS ?11 AND files.mtime IS ?12 AND {GUARD}",
        marks(10)
    )
}

fn marks(n: usize) -> String {
    (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

/// The batch's one `meta.seq` bump, for the folded writes.
fn bump(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("UPDATE meta SET value = value + 1 WHERE key = 'seq' RETURNING value", [], |r| r.get(0))?)
}

fn ids(conn: &Connection) -> Result<Vec<i64>> {
    Ok(conn
        .prepare("SELECT id FROM files ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// The `known` lookup of each file, as the writer makes it before each write: each finds a row when `exists`.
fn known_each(conn: &Connection, ctx: &Ctx, exists: bool) -> Result<Vec<i64>> {
    let mut known = conn.prepare(KNOWN)?;
    let mut found = Vec::with_capacity(ctx.files.len());
    let none: Option<Vec<u8>> = None;
    for f in &ctx.files {
        let id: Option<i64> =
            known.query(params![SERIAL, f.file_ref, b"".to_vec(), none])?.next()?.map(|r| r.get(0)).transpose()?;
        match (id, exists) {
            (Some(id), true) => found.push(id),
            (None, false) => {}
            _ => bail!("the known lookup of {} found {id:?}", f.file_ref),
        }
    }
    Ok(found)
}

/// Every file written by `sql`, one statement each, after its `known` lookup. `seq` is bound as `?11` (the folded
/// insert); `changed` writes each file one byte larger, guarded by its old size and mtime (`?11`, `?12`), with its row's
/// id as `?13`.
fn each_file(conn: &Connection, ctx: &Ctx, sql: &str, seq: Option<i64>, changed: bool) -> Result<()> {
    let mut known = conn.prepare(KNOWN)?;
    let mut write = conn.prepare(sql)?;
    let none: Option<i64> = None;
    let no_bound: Option<Vec<u8>> = None;
    for f in &ctx.files {
        let id: Option<i64> =
            known.query(params![SERIAL, f.file_ref, b"".to_vec(), no_bound])?.next()?.map(|r| r.get(0)).transpose()?;
        ensure!(id.is_some() == changed, "the known lookup of {} found {id:?}", f.file_ref);
        let (folder_id, mid) = *ctx.folders.get(f.folder).context("the file's folder")?;
        let size = if changed { f.size + 1 } else { f.size };
        let (old_size, old_mtime) = (changed.then_some(f.size), changed.then_some(f.mtime));
        let seq_or_old_size: &dyn ToSql = if let Some(seq) = &seq { seq } else { &old_size };
        let values: [&dyn ToSql; 13] = [
            &ctx.volume,
            &f.file_ref,
            &none,
            &folder_id,
            &mid,
            &f.name,
            &f.ext,
            &f.kind,
            &size,
            &f.mtime,
            seq_or_old_size,
            &old_mtime,
            &id,
        ];
        let bound = values.get(..write.parameter_count()).context("more parameters than values")?;
        ensure!(write.execute(bound)? == 1, "one row written");
    }
    Ok(())
}

/// The batch as `json_each` reads it: `[volume, file_ref, folder_id, mid, name (hex), ext, kind, size, mtime]`.
fn batch_json(ctx: &Ctx) -> Result<String> {
    let mut json = String::from("[");
    for (i, f) in ctx.files.iter().enumerate() {
        let (folder_id, mid) = *ctx.folders.get(f.folder).context("the file's folder")?;
        if i > 0 {
            json.push(',');
        }
        write!(
            json,
            "[{},{},{folder_id},{mid},\"{}\",{},{},{},{}]",
            ctx.volume,
            f.file_ref,
            hex(&f.name),
            f.ext,
            f.kind,
            f.size,
            f.mtime,
        )?;
    }
    json.push(']');
    Ok(json)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The connection's lookaside counters since the last call (each read resets it): hits, misses because the
/// allocation was larger than a slot, misses because every slot was taken, and the most slots in use.
#[expect(unsafe_code, reason = "the SQLite C API, called through the probe's own FFI declarations")]
fn lookaside(conn: &Connection) -> Result<[i32; 4]> {
    let ops = [
        ffi::SQLITE_DBSTATUS_LOOKASIDE_HIT,
        ffi::SQLITE_DBSTATUS_LOOKASIDE_MISS_SIZE,
        ffi::SQLITE_DBSTATUS_LOOKASIDE_MISS_FULL,
        ffi::SQLITE_DBSTATUS_LOOKASIDE_USED,
    ];
    let mut out = [0; 4];
    for (slot, op) in out.iter_mut().zip(ops) {
        let (mut current, mut highwater) = (0, 0);
        // SAFETY: the handle is this open connection's, used on this thread and not closed here; the two out
        // pointers are to locals that outlive the call.
        let rc = unsafe { ffi::sqlite3_db_status(conn.handle(), op, &raw mut current, &raw mut highwater, 1) };
        ensure!(rc == ffi::SQLITE_OK, "sqlite3_db_status({op}) returned {rc}");
        *slot = highwater;
    }
    Ok(out)
}
