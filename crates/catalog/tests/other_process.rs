//! A catalog another process has open is not rebuilt, and another process's open of it is fine. The other process is this
//! test binary, run again as a holder that opens the catalog (the `holder` test below, which does nothing unless it is
//! run that way), so the holds are the operating system's, not one process's.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pg_catalog::{Catalog, Versions};
use rusqlite::Connection;

/// This test binary, run as a holder of the catalog at `path`: `open` holds it by opening the catalog, `exclusive` as a
/// rebuild does, `post` opens it and posts [`posted`] to its inbox. It makes `ready` once it has, and keeps the hold
/// until `release` exists.
fn spawn_holder(path: &Path, mode: &str, ready: &Path, release: &Path) -> std::io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "holder"])
        .stdout(Stdio::null())
        .env("PG_HOLDER_PATH", path)
        .env("PG_HOLDER_MODE", mode)
        .env("PG_HOLDER_READY", ready)
        .env("PG_HOLDER_RELEASE", release)
        .spawn()
}

/// Run only by `spawn_holder`: without `PG_HOLDER_PATH` it does nothing.
#[test]
fn holder() {
    let Some(path) = std::env::var_os("PG_HOLDER_PATH") else { return };
    let path = Path::new(&path);
    let mode = std::env::var("PG_HOLDER_MODE").unwrap();
    let _hold: Box<dyn std::any::Any> = if mode == "exclusive" {
        Box::new(pg_dbfile::Live::exclusive(path).unwrap().expect("the holder gets the rebuild's hold"))
    } else {
        let c = Catalog::open(path, pg_platform::native::PATH_RULES).expect("the holder opens the catalog");
        if mode == "post" {
            assert!(c.post_notice(&posted(), 10).unwrap());
        }
        Box::new(c)
    };
    std::fs::write(std::env::var_os("PG_HOLDER_READY").unwrap(), b"").unwrap();
    pg_testkit::wait_for_path(Path::new(&std::env::var_os("PG_HOLDER_RELEASE").unwrap()), Duration::from_secs(60));
}

#[test]
fn a_catalog_open_in_another_process_is_not_rebuilt() {
    let dir: PathBuf = pg_testkit::run_folder("scratch_catalog", "other-process").unwrap();
    let path = dir.join("catalog.sqlite");
    drop(Catalog::prepare(&path, Versions::current(), pg_platform::native::PATH_RULES).unwrap());
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut other = spawn_holder(&path, "open", &ready, &release).unwrap();
    pg_testkit::wait_for_path(&ready, Duration::from_secs(60));

    // Opened beside the other process: both hold it shared.
    drop(Catalog::open(&path, pg_platform::native::PATH_RULES).unwrap());

    // The file is edited on a raw connection, which holds nothing: it now needs a rebuild, which the other process
    // prevents.
    let edited = Connection::open(&path).unwrap();
    edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
    drop(edited);
    let err = Catalog::prepare(&path, Versions::current(), pg_platform::native::PATH_RULES).err().expect("refused");
    assert!(format!("{err:#}").contains("has it open"), "{err:#}");
    assert!(!dir.join("catalog.sqlite.replaced").exists(), "nothing was moved aside");

    std::fs::write(&release, b"").unwrap();
    assert!(other.wait().unwrap().success(), "the holder ends");
    let c = Catalog::prepare(&path, Versions::current(), pg_platform::native::PATH_RULES).unwrap();
    assert!(c.rebuilt().is_some(), "rebuilt once nothing else has it open");
}

/// What the `post` holder posts.
fn posted() -> pg_model::notice::Notice {
    pg_model::notice::Notice::FolderNotAdded { error: "posted by another process".into() }
}

/// A notice another process posts is seen here: the catalog's `data_version` moves (what the app's inbox polls), and
/// the inbox lists it.
#[test]
fn a_notice_another_process_posts_is_seen() {
    let dir: PathBuf = pg_testkit::run_folder("scratch_catalog", "other-process-post").unwrap();
    let path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&path, Versions::current(), pg_platform::native::PATH_RULES).unwrap();
    let before = c.data_version().unwrap();
    assert!(c.inbox_entries(10, false).unwrap().is_empty());
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut other = spawn_holder(&path, "post", &ready, &release).unwrap();
    pg_testkit::wait_for_path(&ready, Duration::from_secs(60));
    assert_ne!(c.data_version().unwrap(), before, "another connection's commit moves it");
    let entries = c.inbox_entries(10, false).unwrap();
    assert_eq!(entries.iter().map(|e| e.notice.clone()).collect::<Vec<_>>(), [posted()]);
    std::fs::write(&release, b"").unwrap();
    assert!(other.wait().unwrap().success(), "the holder ends");
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An open waits for a rebuild another process is making (it holds the catalog exclusively), and gets the catalog once
/// that process lets go.
#[test]
fn open_waits_for_a_rebuild_in_another_process() {
    let dir: PathBuf = pg_testkit::run_folder("scratch_catalog", "open-waits").unwrap();
    let path = dir.join("catalog.sqlite");
    drop(Catalog::prepare(&path, Versions::current(), pg_platform::native::PATH_RULES).unwrap());
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut rebuild = spawn_holder(&path, "exclusive", &ready, &release).unwrap();
    pg_testkit::wait_for_path(&ready, Duration::from_secs(60));

    // The other process lets go a second after the open starts.
    let releaser = std::thread::spawn({
        let release = release.clone();
        move || {
            std::thread::sleep(Duration::from_secs(1));
            std::fs::write(release, b"").unwrap();
        }
    });
    let start = Instant::now();
    let opened = Catalog::open(&path, pg_platform::native::PATH_RULES);
    assert!(opened.is_ok(), "opened once the rebuild let go");
    assert!(start.elapsed() >= Duration::from_secs(1), "it waited for the rebuild");
    releaser.join().unwrap();
    assert!(rebuild.wait().unwrap().success(), "the holder ends");
}
