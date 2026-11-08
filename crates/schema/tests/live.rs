//! The `.live` hold across processes: a shared hold in one process keeps another's exclusive hold out, and a shared hold
//! waits for an exclusive one in another process. The other process is this test binary, run again as a holder (the
//! `holder` test below, which does nothing unless it is run that way), so the holds are the operating system's.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use pg_schema::Live;

/// A folder for one test, made fresh.
fn scratch(name: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("pg-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Waits until `path` exists, for at most `within`.
fn wait_for(path: &Path, within: Duration) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < within, "{} never appeared", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This test binary, run as a holder of the database `db`: it takes the hold `mode` names (`shared` or `exclusive`),
/// makes `ready` once it has it, and keeps it until `release` exists.
fn spawn_holder(db: &Path, mode: &str, ready: &Path, release: &Path) -> std::io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "holder", "--nocapture"])
        .env("PG_LIVE_DB", db)
        .env("PG_LIVE_MODE", mode)
        .env("PG_LIVE_READY", ready)
        .env("PG_LIVE_RELEASE", release)
        .spawn()
}

/// Run only by `spawn_holder`: without `PG_LIVE_DB` it does nothing.
#[test]
fn holder() {
    let Some(db) = std::env::var_os("PG_LIVE_DB") else { return };
    let hold = if std::env::var("PG_LIVE_MODE").unwrap() == "shared" {
        Live::shared(Path::new(&db)).unwrap()
    } else {
        Live::exclusive(Path::new(&db)).unwrap()
    };
    let _hold = hold.expect("the holder gets its hold");
    std::fs::write(std::env::var_os("PG_LIVE_READY").unwrap(), b"").unwrap();
    wait_for(Path::new(&std::env::var_os("PG_LIVE_RELEASE").unwrap()), Duration::from_secs(60));
}

/// A shared hold in another process keeps the exclusive hold out, and shares with other shared holds.
#[test]
fn a_shared_hold_in_another_process_keeps_out_the_exclusive_one() {
    let dir = scratch("shared").unwrap();
    let db = dir.join("catalog.sqlite");
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut holder = spawn_holder(&db, "shared", &ready, &release).unwrap();
    wait_for(&ready, Duration::from_secs(60));

    assert!(Live::exclusive(&db).unwrap().is_none(), "another process holds it shared");
    let mine = Live::shared(&db).unwrap();
    assert!(mine.is_some(), "shared holds are shared");
    drop(mine);

    std::fs::write(&release, b"").unwrap();
    assert!(holder.wait().unwrap().success(), "the holder ends");
    assert!(Live::exclusive(&db).unwrap().is_some(), "free once the other process lets go");
}

/// An exclusive hold in another process is waited for by a shared one, which gets it once that process lets go.
#[test]
fn a_shared_hold_waits_for_an_exclusive_one_in_another_process() {
    let dir = scratch("exclusive").unwrap();
    let db = dir.join("catalog.sqlite");
    let (ready, release) = (dir.join("ready"), dir.join("release"));
    let mut holder = spawn_holder(&db, "exclusive", &ready, &release).unwrap();
    wait_for(&ready, Duration::from_secs(60));

    assert!(Live::exclusive(&db).unwrap().is_none(), "another process holds it exclusively");
    // The other process lets go a second after the wait starts.
    let releaser = std::thread::spawn({
        let release = release.clone();
        move || {
            std::thread::sleep(Duration::from_secs(1));
            std::fs::write(release, b"").unwrap();
        }
    });
    let start = Instant::now();
    let mine = Live::shared(&db).unwrap();
    assert!(mine.is_some(), "granted once the exclusive holder let go");
    assert!(start.elapsed() >= Duration::from_secs(1), "it waited for the exclusive hold");
    releaser.join().unwrap();
    assert!(holder.wait().unwrap().success(), "the holder ends");
}
