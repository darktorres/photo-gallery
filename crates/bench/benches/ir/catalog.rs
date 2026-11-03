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

use pg_catalog::browse::{self, read::Facets};
use pg_model::grid::Entry;
use pg_model::tree::FolderTree;
use pg_analysis::analyze::IMPORT_VERSION;
use pg_analysis::borders::BORDERS_VERSION;
use pg_catalog::{Catalog, Versions, WorkItem};
use pg_model::path::WorkList;
use pg_model::query::{Query, RatingFilter, Sort, SortKey};
use pg_thumbcache::{self as thumbs, Pending};

const DATA: &str = match option_env!("PG_BENCH_DATA") {
    Some(path) => path,
    None => "/root/pg-bench",
};
const LAYOUT: &str = "v1-orderkey-synthetic";
// The thumbnail height the cache was filled at.
const THUMB_HEIGHT: u32 = 256;

fn run_dir() -> PathBuf {
    Path::new(DATA).join("run")
}

// Fresh copies of the layout's files in run/, in WAL mode like the app's.
fn copy_data() -> PathBuf {
    let (from, run) = (Path::new(DATA).join(LAYOUT), run_dir());
    std::fs::create_dir_all(&run).unwrap();
    for name in ["catalog.sqlite", "thumbs.sqlite"] {
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(run.join(format!("{name}{suffix}")));
        }
        std::fs::copy(from.join(name), run.join(name)).unwrap_or_else(|e| panic!("{}: {e}", from.join(name).display()));
        let conn = rusqlite::Connection::open(run.join(name)).unwrap();
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())).unwrap();
    }
    run
}

fn open_catalog() -> Catalog {
    let versions = Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION };
    let catalog = Catalog::prepare(&copy_data().join("catalog.sqlite"), versions).unwrap();
    catalog.status().unwrap(); // reads the schema and the counts: the connection is warm before the measured code
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
    let entries = browse::read::list_grid(&catalog, folder, recursive, &query, sort).unwrap();
    (catalog, black_box(entries))
}

// The folder tree with its counts.
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn tree(catalog: Catalog) -> (Catalog, FolderTree) {
    let tree = browse::read::folder_tree(&catalog).unwrap();
    (catalog, black_box(tree))
}

// The filter bar's counts for "All folders".
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn facets(catalog: Catalog) -> (Catalog, Facets) {
    let facets = browse::read::facets(&catalog, "", true, &Query::default()).unwrap();
    (catalog, black_box(facets))
}

// The import's check for work, every round of the background job (nothing is pending in this catalog).
#[library_benchmark]
#[bench::empty(setup = open_catalog)]
fn pending_import(catalog: Catalog) -> (Catalog, WorkList<WorkItem>) {
    let work = catalog.pending_import(false).unwrap();
    (catalog, black_box(work))
}

// The thumbnail pass's check for work, on its own connection like the job's.
#[library_benchmark]
#[bench::at_height(setup = copy_data)]
fn pending_thumbs(run: PathBuf) -> Pending {
    black_box(thumbs::pending_in(&run.join("thumbs.sqlite"), &run.join("catalog.sqlite"), THUMB_HEIGHT).unwrap())
}

library_benchmark_group!(name = catalog, benchmarks = [list, tree, facets, pending_import, pending_thumbs]);
