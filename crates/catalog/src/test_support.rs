//! What the catalog's tests share: the versions this build stamps, paths in either rules' form, and the catalog
//! fixtures the tests of more than one module use (`core`, `lifecycle` and `writers` each test their own code).

use crate::browse::fixtures::spelled;
use crate::fixture::{self, raw};
use crate::{Catalog, Prior, Recorded, ScanLease, ScannedFile, Versions, WorkItem, queue};
use pg_model::analysis::{Analysis, ImageFacts, Media, Taken};
use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FolderId, ScanId};
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::turns::Turns;
use pg_model::versions::IMPORT_VERSION;
use std::mem::ManuallyDrop;
use std::path::Path;

/// The versions this build's analyses are stamped with, as the library passes them to [`Catalog::prepare`].
pub(crate) fn versions() -> Versions {
    Versions::current()
}

/// A path under `rules`, from its text as the tests write it, in the Windows form (`D:\p\sub`), which [`spelled`]
/// turns into the form of `rules` (`/p/sub` under POSIX).
pub(crate) fn p(rules: PathRules, text: &str) -> RulePath {
    rules.parse(&spelled(rules, text)).expect("a test path")
}

/// An in-memory catalog under `rules` with the library root `D:\r`: the start of the root and removal tests.
pub(crate) fn rooted(rules: PathRules) -> Catalog {
    let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
    assert!(c.add_root(&p(rules, r"D:\r")).unwrap());
    c
}

/// A catalog with one file in it, under `rules`.
pub(crate) fn one_file(rules: PathRules) -> (Catalog, i64) {
    one_file_at(rules, Path::new(":memory:"))
}

/// [`one_file`] at `path` (`:memory:`, or a new file).
pub(crate) fn one_file_at(rules: PathRules, path: &Path) -> (Catalog, i64) {
    let c = Catalog::prepare(path, versions(), rules).unwrap();
    crate::fixture::state::add_scan(&c, 1, &spelled(rules, "D:")).unwrap();
    crate::fixture::state::add_folder(&c, 1, &spelled(rules, r"D:\x")).unwrap();
    fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
    (c, 1)
}

/// A turn the file can't hold, written by the writer: the catalog keeps it.
pub(crate) fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
    let turns = Turns::new(u8::try_from(turns).unwrap());
    let job = queue::Job { file_id: id, write: pg_model::write::Write::Rotation(turns), temp: None, attempts: 0 };
    c.write_done(&job, queue::WriteEffect::KeptInCatalog(turns)).unwrap();
}

/// Whether the import would analyze file `id` now.
pub(crate) fn due(c: &Catalog, id: FileId) -> bool {
    c.pending_import(false, &crate::VolumeSet::All).unwrap().items.iter().any(|i| i.id == id)
}

/// Whether file `id` is due for the import, and its change number (`files.seq`).
pub(crate) fn state(c: &Catalog, id: FileId) -> (bool, i64) {
    (due(c, id), raw::file_seq(c, id))
}

/// A walk of `root` begun: its `scans` row and its lease, which it keeps until its end ([`lease`]), so its writes find
/// it. Written as `Catalog::begin_scan` writes them, without its refusal of an overlapping walk: the tests of the
/// writers begin walks one after another, or several of one folder, without ending them (the lease's own rules are
/// tested through `begin_scan`).
pub(crate) fn begin(c: &mut Catalog, root: &RulePath) -> ScanId {
    let bytes = c.rules().join(root);
    crate::roots::ensure_seen(&c.conn).unwrap();
    // As `begin_scan` does: a walk starts with no rows seen (the previous walk's set went with its lease).
    c.conn.execute("DELETE FROM temp.scan_seen", []).unwrap();
    c.conn.execute("INSERT INTO scans(root, started) VALUES (?1, unixepoch())", [&bytes]).unwrap();
    let scan = ScanId::new(c.conn.last_insert_rowid());
    c.conn
        .execute(
            "INSERT INTO scan_leases(id, root, holder, heartbeat) VALUES (?1, ?2, 'a test', unixepoch())",
            rusqlite::params![scan.get(), bytes],
        )
        .unwrap();
    scan
}

/// The lease of the walk `scan` of `root` ([`begin`]), to run its end through (`pair_after_scan`, `mark_missing`) as
/// the walk does. It is not given back when dropped: the test goes on with the walk, or ends it (`mark_missing` does).
pub(crate) fn lease<'c>(c: &'c mut Catalog, scan: ScanId, root: &RulePath) -> ManuallyDrop<ScanLease<'c>> {
    ManuallyDrop::new(ScanLease::new(c, scan, root.clone()))
}

/// The rows a walk compares `identities` with, the files it listed in `folder` just now: the folder's rows, read
/// before the listing ([`Catalog::folder_rows`]), then the others' by identity ([`Catalog::prior_states`]).
pub(crate) fn listed(c: &Catalog, folder: &RulePath, identities: &[FileIdentity]) -> Vec<Option<Prior>> {
    let rows = c.folder_rows(folder).unwrap();
    c.prior_states(&rows, identities).unwrap()
}

/// The scanner finding file 7 on volume 1 in D:\x like this, its folder listed just before; whether that changed the
/// catalog.
pub(crate) fn scanned(c: &mut Catalog, scan: pg_model::ids::ScanId, name: &str, size: u64, mtime: i64) -> bool {
    let rules = c.rules();
    let identity = FileIdentity::new(1, 7);
    let prior = listed(c, &p(rules, r"D:\x"), &[identity]).remove(0);
    let tx = c.tx().unwrap();
    let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
    let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
    let ext = Ext::of_name(name).unwrap();
    let file = ScannedFile { identity, name: &Name::from(name), ext, size, mtime, created: None, prior };
    let changed = writer.record_file(&folder, &file).unwrap() == Recorded::Changed;
    drop(writer);
    tx.commit().unwrap();
    changed
}

/// The facts of a JPEG with no rating, for the analyses the tests store (the hashes are the Hashes pass's: a test stores
/// them with `store_hashes`).
pub(crate) fn a_jpeg() -> Media {
    Media::Image(ImageFacts { format: Format::Jpeg, file_rating: None })
}

/// The import storing what it made of `item` (a failure: it counts as made, and needs no Analysis).
pub(crate) fn analyzed(c: &mut Catalog, item: &WorkItem) {
    let tx = c.tx().unwrap();
    tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in for an analysis").unwrap();
    tx.commit().unwrap();
}

pub(crate) fn next_work(c: &Catalog) -> WorkItem {
    let mut work = c.pending_import(false, &crate::VolumeSet::All).unwrap();
    assert_eq!(work.len(), 1);
    work.items.remove(0)
}

/// Stores an import of `item` with `taken` as its time, as the import stores it.
pub(crate) fn store_taken(c: &mut Catalog, item: &WorkItem, taken: Option<Taken>) {
    let analysis = Analysis {
        version: IMPORT_VERSION,
        orientation: Orientation::UPRIGHT,
        width: 400,
        height: 300,
        frames: 1,
        taken,
        media: a_jpeg(),
    };
    let tx = c.tx().unwrap();
    tx.analysis_writer(versions().import).unwrap().store_analysis(item, &analysis).unwrap();
    tx.commit().unwrap();
}

/// `analysis.taken` and `analysis.taken_utc` of file 1.
pub(crate) fn taken_columns(c: &Catalog) -> (Option<i64>, Option<i64>) {
    c.conn
        .query_row("SELECT taken, taken_utc FROM analysis WHERE file_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
}

/// The local wall clock of a UTC instant in this system's zone, as SQLite converts it.
pub(crate) fn local_of(c: &Catalog, instant: i64) -> i64 {
    c.conn.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [instant], |r| r.get(0)).unwrap()
}

/// The files whose `files.changed` stamp is after `since`, in id order, and the catalog's change number now (`meta.seq`),
/// read in one snapshot: what a client that kept its list as of `since` would fetch again. The stamps are the schema's
/// triggers; nothing in the app reads them yet; a planned change will.
pub(crate) fn changed_since(c: &Catalog, since: i64) -> (Vec<FileId>, i64) {
    let snapshot = c.conn.unchecked_transaction().unwrap();
    let now: i64 = snapshot.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |r| r.get(0)).unwrap();
    let files = snapshot
        .prepare("SELECT id FROM files WHERE changed > ?1 ORDER BY id")
        .unwrap()
        .query_map([since], |r| Ok(FileId::new(r.get(0)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    (files, now)
}
