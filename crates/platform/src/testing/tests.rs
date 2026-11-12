//! The test folders' protocol: the locks, the cleanup, and the races the seams let a test run at (see the parent module).

use super::seam::{Hook, Hooked, set};
use super::*;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::rc::Rc;

const SCRATCH: &str = "scratch_platform_testing";

/// The environment variable that tells the child process of the cross-process test which prefix to claim under.
const CHILD_PREFIX: &str = "PG_PLATFORM_TESTING_CHILD_PREFIX";

/// A fresh folder for one test, under the test base. The names are `fixture-…`, so no run folder prefix is one of them.
fn fixture(name: &str) -> PathBuf {
    let dir = base(SCRATCH).join(format!("fixture-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Installs `hook` for the test, on this thread, until the guard drops.
fn hook(hook: impl for<'a> FnMut(&Step<'a>) + 'static) -> Hooked {
    let boxed: Hook = Box::new(hook);
    set(boxed)
}

#[test]
fn a_run_name_is_a_folder_of_its_prefix_and_nothing_else() {
    assert!(is_run_name("run", "run-42-7"));
    assert!(is_run_name("run", "run-42"), "an earlier build's folder is one of its prefix");
    assert!(!is_run_name("run", "run-5-42-7"), "a prefix that ends in a digit: another test's folder");
    assert!(!is_run_name("run", "runs-42-7"));
    assert!(!is_run_name("run", "run-roots-7"));
    assert!(!is_run_name("run", "run--7"));
    assert!(!is_run_name("run", "run-42-"));
}

#[test]
fn a_held_name_is_taken_only_once_its_lock_has_dropped() {
    let dir = fixture("held");
    let path = dir.join("held.owner");
    let lock = lock::create_held(&path).unwrap();
    assert_eq!(lock::create_held(&path).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert!(lock::take_for_delete(&path).unwrap().is_none(), "a held name is not taken");
    drop(lock);
    let taken = lock::take_for_delete(&path).unwrap().expect("a name nobody holds is taken");
    taken.delete().unwrap();
    assert!(!path.exists());
}

#[test]
fn a_leftover_folder_is_removed_with_its_lock() {
    let base = fixture("leftover");
    let folder = base.join("left-999999999-0");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file.txt"), b"x").unwrap();
    fs::write(base.join("left-999999999-0.owner"), b"").unwrap();
    sweep(&base, "left");
    assert!(!folder.exists());
    assert!(!base.join("left-999999999-0.owner").exists());
}

#[test]
fn a_folder_with_no_lock_is_removed_with_the_lock_the_cleanup_made() {
    let base = fixture("unlocked");
    let folder = base.join("unlocked-999999999-0");
    fs::create_dir(&folder).unwrap();
    sweep(&base, "unlocked");
    assert!(!folder.exists());
    assert!(!base.join("unlocked-999999999-0.owner").exists());
}

#[test]
fn a_lock_with_no_folder_is_removed_unless_something_holds_it() {
    let base = fixture("lock-only");
    let path = base.join("lonely-999999999-0.owner");
    fs::write(&path, b"").unwrap();
    let held = lock::create_held(&base.join("held-999999999-0.owner")).unwrap();
    sweep(&base, "lonely");
    sweep(&base, "held");
    assert!(!path.exists(), "a lock nothing holds is removed");
    assert!(base.join("held-999999999-0.owner").exists(), "a held lock is kept");
    drop(held);
}

#[test]
fn a_folder_its_lock_holds_is_kept_until_the_lock_is_released() {
    let base = fixture("held-folder");
    let folder = base.join("kept-999999999-0");
    fs::create_dir(&folder).unwrap();
    let lock = lock::create_held(&base.join("kept-999999999-0.owner")).unwrap();
    sweep(&base, "kept");
    assert!(folder.is_dir(), "a folder whose lock is held is kept");
    drop(lock);
    sweep(&base, "kept");
    assert!(!folder.exists());
}

#[test]
fn a_folder_of_another_prefix_is_not_swept() {
    let base = fixture("other-prefix");
    let other = base.join("shared-roots-7");
    fs::create_dir(&other).unwrap();
    sweep(&base, "shared");
    assert!(other.is_dir());
}

#[test]
fn an_earlier_builds_folder_is_kept_while_its_owner_file_is_locked() {
    let base = fixture("old-format");
    let folder = base.join("old-999999999");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join(".owner"), b"").unwrap();
    let owner = fs::OpenOptions::new().read(true).write(true).open(folder.join(".owner")).unwrap();
    owner.lock().unwrap();
    sweep(&base, "old");
    assert!(folder.join(".owner").is_file(), "its process still holds it");
    drop(owner);
    sweep(&base, "old");
    assert!(!folder.exists(), "once its process has ended, the folder is removed");
}

#[test]
fn an_earlier_builds_folder_is_removed_only_under_its_lock() {
    let base = fixture("old-seam");
    let folder = base.join("oldseam-999999999");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join(".owner"), b"").unwrap();
    let lock_path = base.join("oldseam-999999999.owner");
    let refused: Rc<RefCell<Option<io::ErrorKind>>> = Rc::new(RefCell::new(None));
    let seen = refused.clone();
    let _guard = hook(move |step| {
        if matches!(step, Step::OldBeforeRemove(_)) {
            // A claim of the same name, while the cleanup is about to remove the folder: it is refused.
            *seen.borrow_mut() = lock::create_held(&lock_path).err().map(|err| err.kind());
        }
    });
    sweep(&base, "oldseam");
    assert!(!folder.exists());
    assert_eq!(*refused.borrow(), Some(io::ErrorKind::AlreadyExists));
}

#[test]
fn a_claim_made_between_a_cleanups_look_and_its_lock_keeps_the_folder() {
    let base = fixture("race-look");
    let folder = base.join("race-999999999-0");
    fs::create_dir(&folder).unwrap();
    let lock_path = base.join("race-999999999-0.owner");
    let claimed: Rc<RefCell<Option<HeldLock>>> = Rc::new(RefCell::new(None));
    let kept = claimed.clone();
    let _guard = hook(move |step| {
        if matches!(step, Step::SweepBeforeAcquire(_)) {
            // A claim of the same name, as a run makes one: its lock first.
            *kept.borrow_mut() = lock::create_held(&lock_path).ok();
        }
    });
    sweep(&base, "race");
    assert!(claimed.borrow().is_some(), "the claim got its lock");
    assert!(folder.is_dir(), "the cleanup does not remove a folder a claim holds");
}

#[test]
fn a_cleanup_during_each_step_of_a_claim_keeps_the_claim() {
    let base = fixture("claim-steps");
    let root = base.clone();
    let _guard = hook(move |step| {
        if matches!(step, Step::ClaimLocked(_) | Step::ClaimMade(_)) {
            sweep(&root, "steps");
        }
    });
    let folder = run_in(&base, "steps").unwrap();
    drop(_guard);
    assert!(folder.is_dir(), "the claim's folder survives the cleanups made in its steps");
    sweep(&base, "steps");
    assert!(folder.is_dir(), "and a later cleanup keeps it too");
}

#[cfg(unix)]
#[test]
fn a_claim_whose_hidden_name_a_cleanup_takes_starts_over() {
    let base = fixture("hidden");
    let root = base.clone();
    let mut done = false;
    let _guard = hook(move |step| {
        // Once: the cleanup takes the hidden name of the first attempt, and the claim makes the next one.
        if matches!(step, Step::ClaimHidden(_)) && !done {
            done = true;
            sweep(&root, "hidden");
        }
    });
    let folder = run_in(&base, "hidden").unwrap();
    drop(_guard);
    assert!(folder.is_dir());
    let hidden_left =
        fs::read_dir(&base).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with('.'));
    assert!(!hidden_left, "no hidden name is left behind");
}

#[cfg(unix)]
#[test]
fn a_lock_whose_name_was_made_again_is_not_taken() {
    let base = fixture("reuse");
    let path = base.join("reuse-999999999-0.owner");
    fs::write(&path, b"old").unwrap();
    let again = path.clone();
    let mut done = false;
    let _guard = hook(move |step| {
        if matches!(step, Step::TakeLocked(_)) && !done {
            done = true;
            fs::remove_file(&again).unwrap();
            fs::write(&again, b"new").unwrap();
        }
    });
    assert!(lock::take_for_delete(&path).unwrap().is_none(), "the name is another file now");
}

#[cfg(unix)]
#[test]
fn a_claim_made_in_a_cleanups_delete_window_survives() {
    let base = fixture("window");
    let folder = base.join("window-999999999-0");
    fs::create_dir(&folder).unwrap();
    let fresh: Rc<RefCell<Option<HeldLock>>> = Rc::new(RefCell::new(None));
    let kept = fresh.clone();
    let root = base.clone();
    let claim_folder = folder.clone();
    let _guard = hook(move |step| {
        if matches!(step, Step::BeforeClose(_)) {
            // The cleanup has removed the folder and the lock's name: a claim of the same name, then a cleanup of its own.
            let lock = lock::create_held(&root.join("window-999999999-0.owner")).unwrap();
            fs::create_dir(&claim_folder).unwrap();
            *kept.borrow_mut() = Some(lock);
            sweep(&root, "window");
        }
    });
    sweep(&base, "window");
    drop(_guard);
    assert!(folder.is_dir(), "the claim's folder survives the cleanup that removed the first one");
    assert!(fresh.borrow().is_some());
}

#[test]
fn a_folder_another_process_holds_is_kept_until_that_process_ends() {
    let root = base(SCRATCH);
    fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "testing::tests::child_holds_a_run_folder", "--ignored", "--nocapture"])
        .env("PG_TEST_DIR", &root)
        .env(CHILD_PREFIX, "child-held")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let folder = loop {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "the child ended before it held a folder");
        if let Some(path) = line.trim_end().strip_prefix("PG_TESTING_HELD ") {
            break PathBuf::from(path);
        }
    };
    sweep(&root, "child-held");
    assert!(folder.is_dir(), "a folder another process holds is kept");
    // The child reads its stdin to the end, and returns once it has: closing ours ends it.
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    sweep(&root, "child-held");
    assert!(!folder.exists(), "once that process has ended, its folder is removed");
}

/// The child of [`a_folder_another_process_holds_is_kept_until_that_process_ends`]: it claims a folder, says so, and holds
/// it until its stdin ends. Run only by that test.
#[test]
#[ignore = "run by a_folder_another_process_holds_is_kept_until_that_process_ends"]
fn child_holds_a_run_folder() {
    let Ok(prefix) = std::env::var(CHILD_PREFIX) else { return };
    let folder = run_in(&base(SCRATCH), &prefix).unwrap();
    println!("PG_TESTING_HELD {}", folder.display());
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest).unwrap();
}

#[test]
fn run_folders_made_at_once_in_one_process_are_distinct() {
    let root = fixture("threads");
    let folders: Vec<PathBuf> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16).map(|_| s.spawn(|| run_in(&root, "threads").unwrap())).collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect()
    });
    let mut unique = folders.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), folders.len(), "every call has a folder of its own");
    for folder in &folders {
        assert!(folder.is_dir(), "{} survives the other calls", folder.display());
    }
}
