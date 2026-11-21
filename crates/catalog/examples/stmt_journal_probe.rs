//! What the scan's per-file writes cost, and which get a statement journal: `stmt_journal_probe [variant]` runs one
//! variant (or, with none, each in turn) and prints, per variant, its rows, its wall time, and its connection's lookaside
//! counters. For callgrind, run one variant per process and collect its timed part only (`valgrind --tool=callgrind
//! --toggle-collect='stmt_journal_probe::timed_part' target/release/examples/stmt_journal_probe a`). `PG_PROBE_MEMSTAT=0`
//! turns SQLite's memory statistics off before any connection opens.
//!
//! Each variant makes a fresh catalog (`Catalog::prepare`, so the schema and its triggers are the app's), opens it raw
//! with the catalog's connection settings, adds the rows the writes need (two scans rows, 80 folders), changes the
//! schema or the settings as the variant says, then writes 4,000 files in one `BEGIN IMMEDIATE` transaction as
//! `pg_scan::write_batch` does: with `ScanWriter`'s statements prepared and held, and its `known` lookup before each
//! write. The rows are those of the scan probe's library (`folder-NN/IMG_NN_MMMM.jpg` under `/bench-lib`), with order
//! keys made as the scanner makes them, and made-up identities, sizes and times.

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use pg_catalog::{Catalog, Versions};
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_platform::native::PATH_RULES;
use rusqlite::types::ToSql;
use rusqlite::{Connection, Statement, ffi, params};

const FOLDERS: usize = 80;
const PER_FOLDER: usize = 50;
const ROOT: &str = "/bench-lib";
const VOLUME: i64 = 2049;

const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA cache_size = -262144; PRAGMA temp_store = MEMORY; PRAGMA busy_timeout = 5000;";

const GUARD: &str = "NOT EXISTS(SELECT 1 FROM file_writes WHERE file_id = files.id AND temp IS NOT NULL)";
const COLUMNS: &str = "INSERT INTO files(volume, file_ref, created, folder_id, name, ext, kind, size, mtime, last_scan,
    order_key, unpaired";
const DO_UPDATE: &str = "ON CONFLICT(volume, file_ref) DO UPDATE SET
    folder_id = excluded.folder_id, name = excluded.name, ext = excluded.ext, kind = excluded.kind,
    size = excluded.size, mtime = excluded.mtime, last_scan = excluded.last_scan, missing = 0,
    order_key = excluded.order_key, created = coalesce(files.created, excluded.created)";
const KNOWN: &str = "SELECT id, folder_id, name, ext, kind, size, mtime, missing, order_key, created FROM files
     WHERE volume = ?1 AND file_ref = ?2";
/// The rest of `ScanWriter`'s statements, prepared and held through the batch as the writer holds them.
const HELD: [&str; 7] = [
    "SELECT id FROM folders WHERE path_key = ?1",
    "INSERT INTO folders(path, path_key, match_key) VALUES (?1, ?2, ?3) RETURNING id",
    "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?4 AND path_key >= ?1 AND (?2 IS NULL OR path_key < ?2))",
    "UPDATE files SET last_scan = ?3 WHERE volume = ?1 AND file_ref = ?2",
    "UPDATE files SET created = ?2 WHERE id = ?1 AND created IS NULL",
    "UPDATE files SET volume = NULL, file_ref = NULL, missing = 1, unpaired = 1 WHERE id = ?1",
    "UPDATE files SET last_scan = ?2 WHERE id = ?1",
];
const INDEXES: [&str; 8] = [
    "files_folder",
    "files_ref",
    "files_order",
    "files_present",
    "files_to_import",
    "files_fresh",
    "files_seq",
    "files_changed",
];

#[derive(Clone)]
struct File {
    folder: usize,
    file_ref: i64,
    name: Vec<u8>,
    ext: &'static str,
    kind: &'static str,
    size: i64,
    mtime: i64,
    order_key: Vec<u8>,
}

struct Ctx {
    files: Vec<File>,
    folder_ids: Vec<i64>,
    scan: i64,
    rescan: i64,
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
    /// `files_added` as it was up to migration 16, installed: a fresh catalog no longer has it.
    Kept,
}

/// `files_added` as it was up to migration 16 (its macros expanded). The variants that keep, split or drop it install
/// it first, so they measure what they measured before the trigger was removed.
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
        v("e", "seen update (rescan)", true, Removed::Nothing, true, Action::Seen),
        v("e2", "seen update, foreign keys off", false, Removed::Nothing, true, Action::Seen),
        v("e3", "one seen update for the batch", true, Removed::Nothing, true, Action::SeenSet),
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

fn main() -> Result<()> {
    if std::env::var("PG_PROBE_MEMSTAT").is_ok_and(|v| v == "0") {
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

fn run(path: &std::path::Path, variant: &Variant, files: &[File]) -> Result<()> {
    std::mem::drop(Catalog::prepare(path, Versions::current(), PATH_RULES)?);
    let conn = Connection::open(path)?;
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
    let micros = start.elapsed().as_micros();
    let [hit, size, full, used] = lookaside(&conn)?;
    let rows: i64 = conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
    writeln!(
        std::io::stdout(),
        "{}\t{}\t{rows} rows\t{micros} us\tlookaside hits {hit}, misses for size {size}, misses with slots full {full}, \
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
        let folder = PATH_RULES.parse(&format!("{ROOT}/folder-{f:02}")).context("the folder path")?;
        let folder_key = PATH_RULES.folder_key(&folder.parts);
        for n in 1..=PER_FOLDER {
            let text = format!("IMG_{f:02}_{n:04}.jpg");
            let name = Name::from(text.as_str());
            let ext = Ext::of_name_bytes(text.as_bytes()).context("the extension")?;
            let mut order_key = folder_key.clone();
            name.push_name_key(&mut order_key);
            let index = i64::try_from(files.len())?;
            files.push(File {
                folder: f - 1,
                file_ref: 100_000 + index,
                name: text.into_bytes(),
                ext: ext.key(),
                kind: ext.kind().key(),
                size: 1_000 + index,
                mtime: 133_000_000_000_000_000 + index,
                order_key,
            });
        }
    }
    Ok(files)
}

/// Two scans rows (the walk, and a rescan's for the seen updates) and the folders' rows, untimed.
fn setup(conn: &Connection, files: &[File]) -> Result<Ctx> {
    conn.execute("INSERT INTO scans(root, started) VALUES (?1, 0)", [ROOT.as_bytes()])?;
    let scan = conn.last_insert_rowid();
    conn.execute("INSERT INTO scans(root, started) VALUES (?1, 1)", [ROOT.as_bytes()])?;
    let rescan = conn.last_insert_rowid();
    let mut folder_ids = Vec::with_capacity(FOLDERS);
    for f in 1..=FOLDERS {
        let folder = PATH_RULES.parse(&format!("{ROOT}/folder-{f:02}")).context("the folder path")?;
        folder_ids.push(conn.query_row(
            "INSERT INTO folders(path, path_key, match_key) VALUES (?1, ?2, ?3) RETURNING id",
            params![PATH_RULES.join(&folder), PATH_RULES.path_key(&folder), PATH_RULES.folder_key(&folder.parts)],
            |r| r.get(0),
        )?);
    }
    Ok(Ctx { files: files.to_vec(), folder_ids, scan, rescan })
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
        Action::Plain { folded: false } => each_file(conn, ctx, &format!("{COLUMNS}) VALUES ({}, 1)", marks(11)), None, false)?,
        Action::Plain { folded: true } => {
            let seq = bump(conn)?;
            let sql = format!("{COLUMNS}, pending, seq, changed) VALUES ({}, 1, 1, ?12, ?12)", marks(11));
            each_file(conn, ctx, &sql, Some(seq), false)?;
        }
        Action::ChangedUpsert => each_file(conn, ctx, &upsert_sql(), None, true)?,
        Action::ChangedUpdate => each_file(
            conn,
            ctx,
            &format!(
                "UPDATE files SET folder_id = ?4, name = ?5, ext = ?6, kind = ?7, size = ?8, mtime = ?9, last_scan = ?10,
                    missing = 0, order_key = ?11, created = coalesce(created, ?3)
                 WHERE id = ?14 AND size IS ?12 AND mtime IS ?13 AND {GUARD}"
            ),
            None,
            true,
        )?,
        Action::SetUpsert | Action::SetFolded => {
            let folded = matches!(action, Action::SetFolded);
            known_each(conn, ctx, false)?;
            let json = batch_json(ctx)?;
            let select = "SELECT j.value->>0, j.value->>1, NULL, j.value->>2, unhex(j.value->>3), j.value->>4,
                j.value->>5, j.value->>6, j.value->>7, ?2, unhex(j.value->>8), 1";
            let n = if folded {
                let seq = bump(conn)?;
                conn.execute(
                    &format!("{COLUMNS}, pending, seq, changed) {select}, 1, ?3, ?3 FROM json_each(?1) AS j"),
                    params![json, ctx.scan, seq],
                )?
            } else {
                conn.execute(
                    &format!(
                        "{COLUMNS}) {select} FROM json_each(?1) AS j WHERE true {DO_UPDATE}
                         WHERE files.size IS excluded.size AND {GUARD}"
                    ),
                    params![json, ctx.scan],
                )?
            };
            ensure!(n == ctx.files.len(), "{n} rows written");
        }
        Action::Seen => {
            let mut seen = conn.prepare("UPDATE files SET last_scan = ?2 WHERE id = ?1")?;
            for id in ids(conn)? {
                ensure!(seen.execute(params![id, ctx.rescan])? == 1, "one row seen");
            }
        }
        Action::SeenSet => {
            let json = serde_json::to_string(&ids(conn)?)?;
            let n = conn.execute(
                "UPDATE files SET last_scan = ?1 WHERE id IN (SELECT value FROM json_each(?2))",
                params![ctx.rescan, json],
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
        "{COLUMNS}) VALUES ({}, 1) {DO_UPDATE} WHERE files.size IS ?12 AND files.mtime IS ?13 AND {GUARD}",
        marks(11)
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
    for f in &ctx.files {
        let id: Option<i64> = known.query(params![VOLUME, f.file_ref])?.next()?.map(|r| r.get(0)).transpose()?;
        match (id, exists) {
            (Some(id), true) => found.push(id),
            (None, false) => {}
            _ => bail!("the known lookup of {} found {id:?}", f.file_ref),
        }
    }
    Ok(found)
}

/// Every file written by `sql`, one statement each, after its `known` lookup. `seq` is bound as `?12` (the folded
/// insert); `changed` writes each file one byte larger, guarded by its old size and mtime, with its row's id as `?14`.
fn each_file(conn: &Connection, ctx: &Ctx, sql: &str, seq: Option<i64>, changed: bool) -> Result<()> {
    let mut known = conn.prepare(KNOWN)?;
    let mut write = conn.prepare(sql)?;
    let none: Option<i64> = None;
    for f in &ctx.files {
        let id: Option<i64> = known.query(params![VOLUME, f.file_ref])?.next()?.map(|r| r.get(0)).transpose()?;
        ensure!(id.is_some() == changed, "the known lookup of {} found {id:?}", f.file_ref);
        let folder_id = *ctx.folder_ids.get(f.folder).context("the file's folder")?;
        let size = if changed { f.size + 1 } else { f.size };
        let (old_size, old_mtime) = (changed.then_some(f.size), changed.then_some(f.mtime));
        let seq_or_old_size: &dyn ToSql = if let Some(seq) = &seq { seq } else { &old_size };
        let values: [&dyn ToSql; 14] = [
            &VOLUME,
            &f.file_ref,
            &none,
            &folder_id,
            &f.name,
            &f.ext,
            &f.kind,
            &size,
            &f.mtime,
            &ctx.scan,
            &f.order_key,
            seq_or_old_size,
            &old_mtime,
            &id,
        ];
        let bound = values.get(..write.parameter_count()).context("more parameters than values")?;
        ensure!(write.execute(bound)? == 1, "one row written");
    }
    Ok(())
}

/// The batch as `json_each` reads it: `[volume, file_ref, folder_id, name (hex), ext, kind, size, mtime, order key (hex)]`.
fn batch_json(ctx: &Ctx) -> Result<String> {
    let mut json = String::from("[");
    for (i, f) in ctx.files.iter().enumerate() {
        let folder_id = *ctx.folder_ids.get(f.folder).context("the file's folder")?;
        if i > 0 {
            json.push(',');
        }
        write!(
            json,
            "[{VOLUME},{},{folder_id},\"{}\",\"{}\",\"{}\",{},{},\"{}\"]",
            f.file_ref,
            hex(&f.name),
            f.ext,
            f.kind,
            f.size,
            f.mtime,
            hex(&f.order_key)
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
