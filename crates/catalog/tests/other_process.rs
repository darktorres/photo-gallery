//! A catalog another process has open is not rebuilt, and another process's open of it is fine. The other process is this
//! test binary, run again as a holder that opens the catalog (the `holder` test below, which does nothing unless it is
//! run that way), so the holds are the operating system's, not one process's.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use pg_catalog::{Catalog, Versions};
use pg_model::rules::PathRules;
use rusqlite::Connection;

fn versions() -> Versions {
    Versions { import: pg_analysis::analyze::IMPORT_VERSION, borders: pg_analysis::borders::BORDERS_VERSION }
}

/// Waits until `path` exists, for at most `within`.
fn wait_for(path: &Path, within: Duration) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < within, "{} never appeared", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This test binary, run as a holder that opens the catalog at `path`, makes `ready` once it has, and keeps it open
/// until `release` exists.
fn spawn_holder(path: &Path, ready: &Path, release: &Path) -> std::io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "holder", "--nocapture"])
        .env("PG_CATALOG_HOLDER", path)
        .env("PG_CATALOG_READY", ready)
        .env("PG_CATALOG_RELEASE", release)
        .spawn()
}

/// Run only by `spawn_holder`: without `PG_CATALOG_HOLDER` it does nothing.
#[test]
fn holder() {
    let Some(path) = std::env::var_os("PG_CATALOG_HOLDER") else { return };
    let _catalog = Catalog::open(Path::new(&path), PathRules::NATIVE).expect("the holder opens the catalog");
    std::fs::write(std::env::var_os("PG_CATALOG_READY").unwrap(), b"").unwrap();
    wait_for(Path::new(&std::env::var_os("PG_CATALOG_RELEASE").unwrap()), Duration::from_secs(60));
}

#[test]
fn a_catalog_open_in_another_process_is_not_rebuilt() {
    let dir: PathBuf = pg_testkit::test_folder("scratch_catalog", "other-process").unwrap();
    let path = dir.join("catalog.sqlite");
    drop(Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap());
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut other = spawn_holder(&path, &ready, &release).unwrap();
    wait_for(&ready, Duration::from_secs(60));

    // Opened beside the other process: both hold it shared.
    drop(Catalog::open(&path, PathRules::NATIVE).unwrap());

    // The file is edited on a raw connection, which holds nothing: it now needs a rebuild, which the other process
    // prevents.
    let edited = Connection::open(&path).unwrap();
    edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
    drop(edited);
    let err = Catalog::prepare(&path, versions(), PathRules::NATIVE).err().expect("refused");
    assert!(format!("{err:#}").contains("has it open"), "{err:#}");
    assert!(!dir.join("catalog.sqlite.replaced").exists(), "nothing was moved aside");

    std::fs::write(&release, b"").unwrap();
    assert!(other.wait().unwrap().success(), "the holder ends");
    let c = Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap();
    assert!(c.rebuilt().is_some(), "rebuilt once nothing else has it open");
}
