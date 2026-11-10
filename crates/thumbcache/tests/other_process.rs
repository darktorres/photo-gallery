//! A cache another process has open is not started over. The other process is this test binary, run again as a holder that
//! opens the cache (the `holder` test below, which does nothing unless it is run that way), so the holds are the
//! operating system's, not one process's.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_jobs::events::Bus;
use pg_thumbcache::ThumbCache;

/// Waits until `path` exists, for at most `within`.
fn wait_for(path: &Path, within: Duration) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < within, "{} never appeared", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This test binary, run as a holder that opens the cache in `dir` for catalog `catalog_id`, makes `ready` once it has,
/// and keeps it open until `release` exists.
fn spawn_holder(dir: &Path, catalog_id: &str, ready: &Path, release: &Path) -> std::io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "holder"])
        .stdout(Stdio::null())
        .env("PG_THUMB_HOLDER", dir)
        .env("PG_THUMB_CATALOG", catalog_id)
        .env("PG_THUMB_READY", ready)
        .env("PG_THUMB_RELEASE", release)
        .spawn()
}

/// Run only by `spawn_holder`: without `PG_THUMB_HOLDER` it does nothing.
#[test]
fn holder() {
    let Some(dir) = std::env::var_os("PG_THUMB_HOLDER") else { return };
    let catalog = std::env::var("PG_THUMB_CATALOG").unwrap();
    let _cache =
        ThumbCache::open(Path::new(&dir), &catalog, None, 1, Arc::new(Bus::new(1))).expect("the holder opens it");
    std::fs::write(std::env::var_os("PG_THUMB_READY").unwrap(), b"").unwrap();
    wait_for(Path::new(&std::env::var_os("PG_THUMB_RELEASE").unwrap()), Duration::from_secs(60));
}

#[test]
fn a_cache_of_another_catalog_is_not_started_over_while_another_process_has_it_open() {
    let dir = pg_testkit::test_folder("scratch_thumbcache", "other-process").unwrap();
    drop(ThumbCache::open(&dir, "first", None, 1, Arc::new(Bus::new(1))).unwrap());
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut other = spawn_holder(&dir, "first", &ready, &release).unwrap();
    wait_for(&ready, Duration::from_secs(60));

    // Another catalog's cache: it would be started over, which needs the file to be nobody's.
    let err = ThumbCache::open(&dir, "second", None, 1, Arc::new(Bus::new(1))).err().expect("refused");
    assert!(format!("{err:#}").contains("another catalog"), "{err:#}");

    std::fs::write(&release, b"").unwrap();
    assert!(other.wait().unwrap().success(), "the holder ends");
    drop(ThumbCache::open(&dir, "second", None, 1, Arc::new(Bus::new(1))).unwrap());
}
