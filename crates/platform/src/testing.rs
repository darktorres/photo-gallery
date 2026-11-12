//! The test scratch folders. A test gets a folder of its own from [`run_folder`], and the folders that earlier runs left
//! are removed first. The rule the cleanup keeps: a folder is deleted only by whoever holds its lock, and the lock is
//! held before the folder is made, so no cleanup sees a folder in the gap between the two.
//!
//! A folder's lock is the file beside it, `<folder>.owner`. Its holder keeps it open (on Windows unshared; on Unix
//! `flock`ed), so no other open takes it while the holder lives. A lock whose holder has ended is a file nothing holds:
//! the next cleanup takes it, and removes the folder with it.

pub mod held;
mod lock;
mod seam;
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use lock::HeldLock;
use seam::{Step, seam};

/// The suffix of a run folder's lock: `<folder>.owner`, beside the folder.
const LOCK_SUFFIX: &str = ".owner";

/// What a claim's hidden name (Unix) is made of: `.<folder>.owner.claim-<process id>-<n>`, until it is locked and linked.
const CLAIM_INFIX: &str = ".owner.claim-";

/// The owner file an earlier build put inside each folder, locked while its process ran.
const OLD_OWNER: &str = ".owner";

/// The locks of the folders this process has claimed. They stay open until the process ends, so no cleanup takes them.
static HELD: Mutex<Vec<HeldLock>> = Mutex::new(Vec::new());

/// The base folder for a test's files: `PG_TEST_DIR` when it is set, else the temp folder's `scratch` folder. The name
/// is a plain folder name (`"scratch_rate"`): no test writes to a drive it was not told about.
pub fn base(scratch: &str) -> PathBuf {
    match std::env::var_os("PG_TEST_DIR") {
        // `.env` writes paths with forward slashes, and the verbatim (`\\?\`) paths that pg-platform opens are not
        // normalized: a test's folders need the native separator, as the paths the app itself is given have.
        Some(dir) => native_separators(&dir),
        None => system_temp().join(scratch),
    }
}

/// The temp folder. This is the one place test scratch reads it: every other test folder goes through [`run_folder`].
#[expect(clippy::disallowed_methods, reason = "the one read of the temp folder; clippy.toml disallows the rest")]
fn system_temp() -> PathBuf {
    std::env::temp_dir()
}

/// `text` followed by a unit that is no UTF-8, as a name the system can hold: a lone surrogate (`0xD800`) on Windows, a
/// raw `0xFF` byte on Unix. For the tests that a name, a path or an argument is kept exactly.
pub fn odd_name(text: &str) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = text.encode_utf16().collect();
        units.push(0xD800);
        std::ffi::OsString::from_wide(&units)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0xFF);
        std::ffi::OsString::from_vec(bytes)
    }
}

/// A test needs an external tool that is not installed: the test returns after this call. Without `PG_REQUIRE_TOOLS`
/// it prints that it skipped and passes; with it (CI sets it) the test fails, so a missing tool can't pass silently.
#[expect(clippy::print_stderr, reason = "a skipped test says so on stderr, where the test runner prints it")]
pub fn missing_tool(tool: &str) {
    assert!(std::env::var_os("PG_REQUIRE_TOOLS").is_none(), "{tool} not found, and PG_REQUIRE_TOOLS is set");
    eprintln!("{tool} not found: skipped");
}

/// A test that can't check what it is for in this environment (the reason says what) returns after this call: it
/// passes, with the reason on stderr where the test runner prints it.
#[expect(clippy::print_stderr, reason = "a skipped test says so on stderr, where the test runner prints it")]
pub fn skip(reason: &str) {
    eprintln!("skipped: {reason}");
}

/// `dir` with its forward slashes made the native separator. On Windows this is done on the UTF-16 units, so every other
/// unit is kept, a lone surrogate included. On Unix a forward slash is already the separator: the path is kept as it is.
#[cfg(windows)]
fn native_separators(dir: &OsStr) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> =
        dir.encode_wide().map(|unit| if unit == u16::from(b'/') { u16::from(b'\\') } else { unit }).collect();
    PathBuf::from(std::ffi::OsString::from_wide(&units))
}

/// `dir` as it is: on Unix a forward slash is the separator, and every other byte is kept.
#[cfg(not(windows))]
fn native_separators(dir: &OsStr) -> PathBuf {
    PathBuf::from(dir)
}

/// A fresh folder for one test (or one part of a test) in a test binary's run: `prefix-<process id>-<n>` under
/// `base(scratch)`. The run folders of `prefix` that no running process holds are removed first (the ones earlier runs
/// left, and any that have ended); a folder another running process holds is kept, so two test binaries can share a
/// base. Fails if it can't be made.
pub fn run_folder(scratch: &str, prefix: &str) -> io::Result<PathBuf> {
    run_in(&base(scratch), prefix)
}

/// [`run_folder`] in the temp folder itself, for a test that needs the system's volume (see the `sqlite` test).
pub fn run_folder_on_system_temp(prefix: &str) -> io::Result<PathBuf> {
    run_in(&system_temp(), prefix)
}

/// The error for a folder that can't be made or claimed: it names the folder, and how to choose another.
fn in_folder(base: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("the test folder {} ({err}): PG_TEST_DIR names another folder", base.display()))
}

fn run_in(base: &Path, prefix: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(base).map_err(|err| in_folder(base, err))?;
    sweep(base, prefix);
    let pid = std::process::id();
    for _ in 0..lock::ATTEMPTS {
        let n = lock::next_number();
        let name = format!("{prefix}-{pid}-{n}");
        let folder = base.join(&name);
        let lock_path = base.join(format!("{name}{LOCK_SUFFIX}"));
        let lock = match lock::create_held(&lock_path) {
            Ok(lock) => lock,
            // Another run's claim (or a leftover not yet swept) has the name: the next number.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(in_folder(base, err)),
        };
        seam(Step::ClaimLocked(&lock_path));
        match fs::create_dir(&folder) {
            Ok(()) => {}
            // A folder with no lock is a leftover. The lock is ours, so the leftover is replaced.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                fs::remove_dir_all(&folder).map_err(|err| in_folder(base, err))?;
                fs::create_dir(&folder).map_err(|err| in_folder(base, err))?;
            }
            Err(err) => return Err(in_folder(base, err)),
        }
        seam(Step::ClaimMade(&folder));
        HELD.lock().unwrap_or_else(PoisonError::into_inner).push(lock);
        return Ok(folder);
    }
    Err(io::Error::other(format!(
        "no free run folder of {prefix} in {} after {} tries",
        base.display(),
        lock::ATTEMPTS
    )))
}

/// What a name in the base folder is, to the cleanup.
enum Kind {
    /// A run folder.
    Folder,
    /// A run folder's lock.
    Lock,
    /// A claim's hidden name (Unix), before its lock is linked.
    Hidden,
}

/// Whether `name` is a run folder of `prefix`: `prefix-<process id>-<n>`, or an earlier build's `prefix-<process id>`.
/// Digits only, so a folder of a longer prefix, or of this prefix with a digit after it, is another test's.
fn is_run_name(prefix: &str, name: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix).and_then(|rest| rest.strip_prefix('-')) else { return false };
    let parts: Vec<&str> = rest.split('-').collect();
    (1..=2).contains(&parts.len())
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn classify(prefix: &str, name: &str, is_dir: bool) -> Option<Kind> {
    if is_dir {
        return is_run_name(prefix, name).then_some(Kind::Folder);
    }
    if let Some(folder) = name.strip_suffix(LOCK_SUFFIX) {
        return is_run_name(prefix, folder).then_some(Kind::Lock);
    }
    let (folder, _claim) = name.strip_prefix('.')?.split_once(CLAIM_INFIX)?;
    is_run_name(prefix, folder).then_some(Kind::Hidden)
}

/// The cleanup: removes the folders of `prefix` that no running process holds, with their locks, and the claims that
/// were left. Errors are not reported: a folder that can't be removed is left for the next cleanup.
fn sweep(base: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(base) else { return };
    let (mut folders, mut locks, mut hidden) = (Vec::new(), Vec::new(), Vec::new());
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        match classify(prefix, &name, is_dir) {
            Some(Kind::Folder) => folders.push(name),
            Some(Kind::Lock) => locks.push(name),
            Some(Kind::Hidden) => hidden.push(name),
            None => {}
        }
    }
    for name in &folders {
        let folder = base.join(name);
        let lock_path = base.join(format!("{name}{LOCK_SUFFIX}"));
        seam(Step::SweepBeforeAcquire(&lock_path));
        // A lock another process holds (or that a claim is making) is not ours: the folder is kept.
        if let Ok(Some(lock)) = lock::acquire_for_delete(&lock_path) {
            remove_folder(&folder, lock);
        }
    }
    // A lock whose folder is gone: a claim that never made its folder, or one this cleanup just removed (a folder it kept
    // has no lock left, and one that exists now has a claim's lock, which is not this lock's to remove).
    for name in &locks {
        let Some(folder) = name.strip_suffix(LOCK_SUFFIX) else { continue };
        if base.join(folder).exists() {
            continue;
        }
        if let Ok(Some(lock)) = lock::take_for_delete(&base.join(name)) {
            let _ = lock.delete();
        }
    }
    for name in &hidden {
        if let Ok(Some(lock)) = lock::take_for_delete(&base.join(name)) {
            let _ = lock.delete();
        }
    }
}

/// Removes `folder`, whose lock `lock` is ours, then the lock. An earlier build kept an owner file inside its folder,
/// locked while its process ran: taken, that process has ended, and the owner goes with the folder; held, the folder is
/// kept.
fn remove_folder(folder: &Path, lock: HeldLock) {
    match lock::take_for_delete(&folder.join(OLD_OWNER)) {
        Ok(Some(owner)) => {
            clear_except(folder, OLD_OWNER);
            let _ = owner.delete();
            seam(Step::OldBeforeRemove(folder));
            let _ = fs::remove_dir(folder);
        }
        // Held by the earlier build's process: keep the folder.
        Ok(None) => {}
        // No owner file: a folder of this build, whose lock is the only guard.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let _ = fs::remove_dir_all(folder);
        }
        Err(_) => {}
    }
    let _ = lock.delete();
}

/// Removes what is in `folder` but for the file named `keep`.
fn clear_except(folder: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if entry.file_name().as_os_str() == OsStr::new(keep) {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let _ = if is_dir { fs::remove_dir_all(entry.path()) } else { fs::remove_file(entry.path()) };
    }
}
