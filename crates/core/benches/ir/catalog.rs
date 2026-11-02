// The catalog's work for the grid, the tree and the background job, on a copy of a real catalog (189,518 files,
// 690 folders, the pinterest dataset; included from linux.rs). The data is prepared once by
// tools/profiling/wsl-bench-data.sh into PG_BENCH_DATA/<LAYOUT>/: the catalog and a cache without its JPEGs, each
// compacted with VACUUM INTO. Setup copies both to PG_BENCH_DATA/run/ and opens the catalog, so the measured
// function starts with the schema parsed and every page cold, like the app's first list.
//
// The bench ids stay the same across commits; the bodies follow the API. LAYOUT names the catalog layout the code
// expects, and changes in the same commit as the schema.

use std::path::{Path, PathBuf};

use pg_core::browse::{self, Entry, Facets, FolderTree};
use pg_core::catalog::{Catalog, WorkItem};
use pg_core::paths::WorkList;
use pg_core::query::{Query, Sort, SortKey};
use pg_core::thumbs::{self, Pending};

const DATA: &str = match option_env!("PG_BENCH_DATA") {
    Some(path) => path,
    None => "/root/pg-bench",
};
const LAYOUT: &str = "v1-orderkey";
// The thumbnail height the cache was filled at.
const THUMB_HEIGHT: u32 = 220;

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
    let catalog = Catalog::open(&copy_data().join("catalog.sqlite")).unwrap();
    catalog.conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get::<_, i64>(0)).unwrap();
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
// files; one folder with subfolders, recursive; one large folder alone.
#[library_benchmark]
#[bench::all_name(args = (List::AllName), setup = for_list)]
#[bench::all_taken(args = (List::AllTaken), setup = for_list)]
#[bench::all_unrated(args = (List::AllUnrated), setup = for_list)]
#[bench::folder_recursive(args = (List::FolderRecursive), setup = for_list)]
#[bench::folder_flat(args = (List::FolderFlat), setup = for_list)]
fn list((catalog, which): (Catalog, List)) -> (Catalog, Vec<Entry>) {
    let name = Sort::default();
    let unrated = Query { ratings: vec![0], ..Query::default() };
    let (folder, recursive, query, sort) = match which {
        List::AllName => ("", true, Query::default(), name),
        List::AllTaken => ("", true, Query::default(), Sort { key: SortKey::Taken, descending: true }),
        List::AllUnrated => ("", true, unrated, name),
        List::FolderRecursive => (r"PhotoGallery\datasets\pinterest\_todo", true, Query::default(), name),
        List::FolderFlat => (r"PhotoGallery\datasets\pinterest\_incoming", false, Query::default(), name),
    };
    let entries = browse::list_grid(&catalog.conn, folder, recursive, &query, sort).unwrap();
    (catalog, black_box(entries))
}

// The folder tree with its counts.
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn tree(catalog: Catalog) -> (Catalog, FolderTree) {
    let tree = browse::folder_tree(&catalog.conn).unwrap();
    (catalog, black_box(tree))
}

// The filter bar's counts for "All folders".
#[library_benchmark]
#[bench::all(setup = open_catalog)]
fn facets(catalog: Catalog) -> (Catalog, Facets) {
    let facets = browse::facets(&catalog.conn, "", true, &Query::default()).unwrap();
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
