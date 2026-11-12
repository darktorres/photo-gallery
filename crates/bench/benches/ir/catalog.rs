// The catalog's work for the grid, the tree and the background job, on a copy of a synthetic catalog (4,000 files,
// 81 folders: tools/profiling/synthetic_library.py, scanned, imported and thumbnailed at 256 px; included from
// linux.rs). The data is prepared once by tools/profiling/bench_data.py into PG_BENCH_DATA/<LAYOUT>/: the catalog and
// a cache without its JPEGs, each compacted with VACUUM INTO. Setup copies both to PG_BENCH_DATA/run/ and opens the
// catalog, so the measured function starts with the schema parsed and every page cold, like the app's first list.
//
// The bench ids stay the same across commits; the bodies follow the API. LAYOUT names the catalog layout the code
// expects, and changes in the same commit as the schema. It also names the dataset, because bench_data.py never
// overwrites a layout's folder.

use std::path::{Path, PathBuf};

use pg_analysis::analyze::IMPORT_VERSION;
use pg_analysis::borders::BORDERS_VERSION;
use pg_catalog::browse::read::facets::Facets;
use pg_catalog::ledger::Summary;
use pg_catalog::{Catalog, Versions, WorkItem};
use pg_jobs::control::Control;
use pg_model::grid::Entry;
use pg_model::path::WorkList;
use pg_model::query::{Query, RatingFilter, Sort, SortKey};
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_model::tree::FolderTree;
use pg_scan::ScanStats;
use pg_scan::synthetic::SyntheticTree;
use pg_thumbcache::{self as thumbs, Pending};

const DATA: &str = match option_env!("PG_BENCH_DATA") {
    Some(path) => path,
    None => "/root/pg-bench",
};
// The layout: a catalog made on Windows by this build's CLI passes (scan, import, thumbs at 256) from the synthetic
// library at D:\PhotoGallery\synthetic\library. Its paths are Windows paths, as the folder literals below are. The
// name is bumped whenever the schema changes, so data made before it is never read as current. v6 is v5's catalog
// opened once by the build of schema version 7 (migrations 2 to 7: the borders' change stamps, the inbox, the ledger,
// the session row, the thumbnail views, the typed queue), so its rows are v5's and a change between the two is the
// code's; the cache's schema is unchanged. v7 is v6's catalog opened once by the build of schema version 8 (the
// notifications' migration), the same way.
const LAYOUT: &str = "v7-windows-rules";
// The large layout (tools/profiling/bench_data.py, `-large`): 200,000 files of the synthetic library's large profile,
// 50,000 of them broken (the import's failures), and the failure ledger seeded to 100,000 rows. Made and named as
// LAYOUT is; the inbox's refresh is measured on it. v6-large was made by the build of schema version 7 (the F0
// baseline's), and v7-large is v6-large's catalog opened once by the build of schema version 8, so both hold the same
// failures.
const LARGE_LAYOUT: &str = "v7-windows-rules-large";
// The path rules the layout was made under (its meta.path_rules). Not pg_platform::native::PATH_RULES: the benches
// run on Linux, where that is posix, and a catalog keeps the rules it was made with.
const RULES: pg_model::rules::PathRules = pg_model::rules::PathRules::Windows;
// The thumbnail height the cache was filled at.
const THUMB_HEIGHT: u32 = 256;

fn run_dir() -> PathBuf {
    Path::new(DATA).join("run")
}

// Fresh copies of the layout's files in run/, in WAL mode like the app's.
fn copy_data() -> PathBuf {
    copy_layout(LAYOUT)
}

// Fresh copies of `layout`'s files in run/.
fn copy_layout(layout: &str) -> PathBuf {
    let (from, run) = (Path::new(DATA).join(layout), run_dir());
    or_stop(std::fs::create_dir_all(&run), run.display());
    for name in ["catalog.sqlite", "thumbs.sqlite"] {
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(run.join(format!("{name}{suffix}")));
        }
        or_stop(std::fs::copy(from.join(name), run.join(name)), from.join(name).display());
        let conn = or_stop(rusqlite::Connection::open(run.join(name)), "the copied catalog");
        or_stop(conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())), "WAL mode");
    }
    run
}

fn versions() -> Versions {
    Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION, thumbs: 1 }
}

fn open_catalog() -> Catalog {
    open_layout(LAYOUT)
}

fn open_layout(layout: &str) -> Catalog {
    let catalog =
        or_stop(Catalog::prepare(&copy_layout(layout).join("catalog.sqlite"), versions(), RULES), "the catalog");
    // A catalog that doesn't match this build's schema is moved aside and an empty one made: the bench would measure
    // nothing. Regenerate the data instead.
    assert!(
        catalog.rebuilt().is_none(),
        "the {layout} data doesn't match this build's schema: regenerate it with tools/profiling/bench_data.py"
    );
    or_stop(catalog.status(), "the catalog's status"); // reads the schema and the counts: the connection is warm before the measured code
    catalog
}

#[derive(Clone, Copy)]
enum List {
    AllName,
    AllTaken,
    AllUnrated,
    FolderRecursive,
    FolderFlat,
}

fn for_list(which: List) -> (Catalog, List) {
    (open_catalog(), which)
}

// The grid's list: "All folders" (the session at startup), by name, by date taken and filtered to the unrated
// files; the library folder with its subfolders, recursive; one subfolder alone (50 files: the synthetic library has
// no large flat folder).
#[library_benchmark]
#[bench::all_name(args = (List::AllName), setup = for_list)]
#[bench::all_taken(args = (List::AllTaken), setup = for_list)]
#[bench::all_unrated(args = (List::AllUnrated), setup = for_list)]
#[bench::folder_recursive(args = (List::FolderRecursive), setup = for_list)]
#[bench::folder_flat(args = (List::FolderFlat), setup = for_list)]
fn list((catalog, which): (Catalog, List)) -> (Catalog, Vec<Entry>) {
    let name = Sort::default();
    let unrated = Query { ratings: vec![RatingFilter::Unrated], ..Query::default() };
    let (folder, recursive, query, sort) = match which {
        List::AllName => ("", true, Query::default(), name),
        List::AllTaken => ("", true, Query::default(), Sort { key: SortKey::Taken, descending: true }),
        List::AllUnrated => ("", true, unrated, name),
        List::FolderRecursive => (r"PhotoGallery\synthetic\library", true, Query::default(), name),
        List::FolderFlat => (r"PhotoGallery\synthetic\library\folder-00", false, Query::default(), name),
    };
    // The layout's folders are literals: a path that does not parse is a bug in the bench.
    #[expect(clippy::panic, reason = "a folder literal that does not parse is a bug in the bench")]
    let Some(parsed) = RULES.parse(folder) else { panic!("a folder path of the layout: {folder}") };
    let folder = RULES.merge(&parsed);
    let entries = or_stop(catalog.list_grid(&folder, recursive, &query, sort), "the grid's list");
    (catalog, black_box(entries))
}

// The folder tree with its counts.
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn tree(catalog: Catalog) -> (Catalog, FolderTree) {
    let tree = or_stop(catalog.folder_tree(), "the folder tree");
    (catalog, black_box(tree))
}

// The filter bar's counts for "All folders".
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn facets(catalog: Catalog) -> (Catalog, Facets) {
    let facets = or_stop(
        catalog.facets(&pg_model::rules::Merged::default(), true, &Query::default()),
        "the facets",
    );
    (catalog, black_box(facets))
}

// The import's check for work, every round of the background job (nothing is pending in this catalog).
#[library_benchmark]
#[bench::empty(setup = open_catalog)]
fn pending_import(catalog: Catalog) -> (Catalog, WorkList<WorkItem>) {
    let work = or_stop(catalog.pending_import(false), "the import's check");
    (catalog, black_box(work))
}

// The thumbnail pass's check for work, on its own connection like the job's.
#[library_benchmark]
#[bench::at_height(setup = copy_data)]
fn pending_thumbs(run: PathBuf) -> Pending {
    // The bench's height is a literal: one the thumbnail cache does not hold is a bug in the bench.
    #[expect(clippy::panic, reason = "a height literal outside the cache's range is a bug in the bench")]
    let Some(height) = ThumbHeight::new(THUMB_HEIGHT) else {
        panic!("the bench's height {THUMB_HEIGHT} is out of range")
    };
    black_box(or_stop(
        thumbs::pending_in(&run.join("thumbs.sqlite"), &run.join("catalog.sqlite"), height),
        "the thumbnail pass's check",
    ))
}

// The files of one import batch (pg-import's 500), each made due again (`Catalog::mark_pending`, as `import --again`
// does), and the analysis the batch stores for each: the sample's. On the large layout, so the import ledger's triggers
// run over its 50,000 failures.
const IMPORT_BATCH: usize = 500;

fn for_import_flush() -> (Catalog, Vec<WorkItem>, Analyzed) {
    let catalog = open_layout(LARGE_LAYOUT);
    let roots = or_stop(catalog.roots(), "the library folders");
    #[expect(clippy::panic, reason = "a layout with no library folder is a bug in the bench data")]
    let Some(root) = roots.first() else { panic!("the {LARGE_LAYOUT} catalog has no library folder") };
    let mut ids = or_stop(catalog.files_under(&root.path), "the library's files");
    ids.truncate(IMPORT_BATCH);
    or_stop(catalog.mark_pending(&ids), "the files made due");
    let mut items = or_stop(catalog.pending_import(false), "the import's work").items;
    items.truncate(IMPORT_BATCH);
    let mut analyzer = or_stop(Analyzer::new(), "the analyzer");
    let analyzed = or_stop(analyzer.analyze(&read_sample(), Turns::NONE), "the sample's analysis");
    (catalog, items, analyzed)
}

// One batch of the import stored (pg-import's flush): its analyses and failures (one in five), in one transaction, with
// the triggers the catalog keeps on them (the import's failure ledger among them).
#[library_benchmark]
#[bench::batch(setup = for_import_flush)]
fn import_flush((mut catalog, items, analyzed): (Catalog, Vec<WorkItem>, Analyzed)) -> Catalog {
    let tx = or_stop(catalog.tx(), "the batch's transaction");
    {
        let mut writer = or_stop(tx.analysis_writer(IMPORT_VERSION), "the analysis writer");
        for (i, item) in items.iter().enumerate() {
            let stored = if i % 5 == 4 {
                writer.store_failure(item, "bench: the analysis failed")
            } else {
                writer.store_analysis(item, &analyzed)
            };
            black_box(or_stop(stored, "a stored analysis"));
        }
    }
    or_stop(tx.commit(), "the batch's commit");
    catalog
}

fn for_refresh(viewing: bool) -> (Catalog, bool) {
    (open_layout(LARGE_LAYOUT), viewing)
}

// The inbox's refresh of the failure ledgers on the large layout (`Catalog::ledger_entries`: each ledger's cheap state,
// `Catalog::ledger_state`), with the inbox closed, and open (the counts read too).
#[library_benchmark]
#[bench::closed(args = (false), setup = for_refresh)]
#[bench::open(args = (true), setup = for_refresh)]
fn inbox_refresh((catalog, viewing): (Catalog, bool)) -> (Catalog, Vec<Summary>) {
    let entries = or_stop(catalog.ledger_entries(viewing), "the ledgers' entries");
    (catalog, black_box(entries))
}

library_benchmark_group!(
    name = catalog,
    benchmarks = [list, tree, facets, pending_import, pending_thumbs, import_flush, inbox_refresh]
);

// The scan's walk (pg-scan's `bench` feature: `scan_with` over a synthetic tree, no file system under it), into a
// catalog in memory with the tree's folder as a library folder: 100 folders of 100 files, and 4 more it can't open
// (the folder ledger's rows). `first` records every file; `again` walks the same tree over the catalog that has it, as
// a rescan does (every file seen, none changed). No bench data: it runs wherever the benches do.
const SCAN_FOLDERS: usize = 100;
const SCAN_FILES: usize = 100;
const SCAN_UNREADABLE: usize = 4;

fn scan_root() -> RulePath {
    #[expect(clippy::panic, reason = "a folder literal that does not parse is a bug in the bench")]
    let Some(root) = RULES.parse(r"D:\bench\library") else { panic!("the synthetic library's folder") };
    root
}

fn synthetic_tree(root: &RulePath) -> SyntheticTree {
    SyntheticTree::new(RULES, root, SCAN_FOLDERS, SCAN_FILES, SCAN_UNREADABLE)
}

fn for_scan_walk(again: bool) -> (Catalog, RulePath, SyntheticTree, Control) {
    let mut catalog = or_stop(Catalog::prepare(Path::new(":memory:"), versions(), RULES), "a catalog in memory");
    let root = scan_root();
    or_stop(catalog.add_root(&root), "the library folder");
    if again {
        or_stop(pg_scan::scan_with(&mut catalog, &root, &Control::new(), &mut synthetic_tree(&root)), "the first walk");
    }
    let tree = synthetic_tree(&root);
    (catalog, root, tree, Control::new())
}

#[library_benchmark]
#[bench::first(args = (false), setup = for_scan_walk)]
#[bench::again(args = (true), setup = for_scan_walk)]
fn scan_walk((mut catalog, root, mut tree, ctl): (Catalog, RulePath, SyntheticTree, Control)) -> (Catalog, ScanStats) {
    let stats = or_stop(pg_scan::scan_with(&mut catalog, &root, &ctl, &mut tree), "the walk");
    (catalog, black_box(stats))
}

library_benchmark_group!(name = scan, benchmarks = [scan_walk]);
