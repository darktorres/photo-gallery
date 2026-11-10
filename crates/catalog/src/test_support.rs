//! What the catalog's tests share: the versions this build stamps, paths in either rules' form, and the catalog
//! fixtures the tests of more than one module use (`core`, `lifecycle` and `writers` each test their own code).

use crate::browse::fixtures::spelled;
use crate::fixture::{self, raw};
use crate::{Catalog, Recorded, ScannedFile, Versions, WorkItem, writes};
use pg_analysis::analyze::IMPORT_VERSION;
use pg_analysis::borders::BORDERS_VERSION;
use pg_model::analysis::{Analysis, Analyzed, ImageFacts, Media, PixelHashes, Taken, ThumbHash};
use pg_model::format::Format;
use pg_model::ids::{FileId, FileIdentity, FolderId};
use pg_model::media::Ext;
use pg_model::name::Name;
use pg_model::orientation::Orientation;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_model::turns::Turns;
use std::path::Path;

/// The versions this build's analyses are stamped with, as the library passes them to [`Catalog::prepare`].
pub(crate) fn versions() -> Versions {
    Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION, thumbs: pg_render::THUMBS_VERSION }
}

/// A path under `rules`, from its text as the tests write it, in the Windows form (`D:\p\sub`), which [`spelled`]
/// turns into the form of `rules` (`/p/sub` under POSIX).
pub(crate) fn p(rules: PathRules, text: &str) -> RulePath {
    rules.parse(&spelled(rules, text)).expect("a test path")
}

/// A catalog with one file in it, under `rules`.
pub(crate) fn one_file(rules: PathRules) -> (Catalog, i64) {
    let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
    raw::add_scan(&c, 1, &spelled(rules, "D:"));
    raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
    fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
    (c, 1)
}

/// A turn the file can't hold, written by the writer: the catalog keeps it.
pub(crate) fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
    let job =
        writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: turns, temp: None, attempts: 0 };
    writes::done(c, &job, writes::WriteEffect::KeptInCatalog(Turns::new(u8::try_from(turns).unwrap()))).unwrap();
}

/// Whether the import would analyze file `id` now.
pub(crate) fn due(c: &Catalog, id: FileId) -> bool {
    c.pending_import(false).unwrap().items.iter().any(|i| i.id == id)
}

/// Whether file `id` is due for the import, and its change number (`files.seq`).
pub(crate) fn state(c: &Catalog, id: FileId) -> (bool, i64) {
    (due(c, id), raw::file_seq(c, id))
}

/// The scanner finding file 7 on volume 1 in D:\x like this; whether that changed the catalog.
pub(crate) fn scanned(c: &mut Catalog, scan: i64, name: &str, size: u64, mtime: i64) -> bool {
    let rules = c.rules();
    let tx = c.tx().unwrap();
    let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
    let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
    let ext = Ext::of_name(name).unwrap();
    let file =
        ScannedFile { identity: FileIdentity::new(1, 7), name: &Name::from(name), ext, size, mtime, created: None };
    let changed = writer.upsert_file(&folder, &file).unwrap() == Recorded::Changed;
    drop(writer);
    tx.commit().unwrap();
    changed
}

/// The facts of a JPEG with no rating and stand-in hashes, for the analyses the tests store.
pub(crate) fn a_jpeg() -> Media {
    let pdq = pg_model::pdq::Pdq { hash: pg_model::pdq::Hash([0; 32]), quality: 0 };
    let pixels = PixelHashes { pixel_hash: [0; 16], pdq };
    Media::Image(ImageFacts { format: Format::Jpeg, file_rating: None, pixels })
}

/// An analysis as the import holds it: an image's dihedral hashes are stand-ins (`a_jpeg`'s), a video's are none.
pub(crate) fn stored_analysis(record: &Analysis) -> Analyzed {
    let pdq_dihedral = record.image().map(|_| [pg_model::pdq::Hash([0; 32]); 8]);
    Analyzed { record: record.clone(), pdq_dihedral }
}

/// The import storing what it made of `item` (a failure: it counts as made, and needs no Analysis).
pub(crate) fn analyzed(c: &mut Catalog, item: &WorkItem) {
    let tx = c.tx().unwrap();
    tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in for an analysis").unwrap();
    tx.commit().unwrap();
}

pub(crate) fn next_work(c: &Catalog) -> WorkItem {
    let mut work = c.pending_import(false).unwrap();
    assert_eq!(work.len(), 1);
    work.items.remove(0)
}

/// Stores an import of `item` with `taken` as its time, as the import stores it.
pub(crate) fn store_taken(c: &mut Catalog, item: &WorkItem, taken: Option<Taken>) {
    let analysis = Analysis {
        version: IMPORT_VERSION,
        file_hash: [1; 32],
        orientation: Orientation::UPRIGHT,
        width: 400,
        height: 300,
        thumbhash: ThumbHash::default(),
        frames: 1,
        taken,
        media: a_jpeg(),
    };
    let tx = c.tx().unwrap();
    tx.analysis_writer(versions().import).unwrap().store_analysis(item, &stored_analysis(&analysis)).unwrap();
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
