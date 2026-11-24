//! Faults the tests put in the writer ([`Writer::start_with_fault`](crate::Writer::start_with_fault)), with the
//! `fault` feature (or in this crate's own tests).

use pg_model::ids::FileId;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

/// Where the writer fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// In a job of this file, as a damaged file the edit can't take would: the job fails, and the writer goes on.
    PanicInProcess(FileId),
    /// In the drain, outside any job (where it reads the queue): the writer thread ends.
    PanicInDrain,
    /// Once, in a job of `file`: another program saves the file as `with`, after the writer read it and before it
    /// opens it for the copy.
    SaveBeforeCopy {
        /// The file whose save is replaced.
        file: FileId,
        /// The bytes the other program saves the file with.
        with: &'static [u8],
    },
    /// Once, in a job of `file`: the job's row leaves the queue after the writer read it and opened the original for
    /// the copy, before the temp is recorded (a change that cancels the write does that, as a rating set back to what
    /// the file holds does).
    ForgetBeforeRecord(FileId),
    /// Every copy of this file fails once its temp is recorded (the copy into the original fails, as with a disk
    /// full), for a fresh job and a finishing copy alike: the copy is tried up to the cap.
    FailCopy(FileId),
    /// In a job of `file`: the job waits at `gate` at the start of its copy into the original, after the temp is recorded,
    /// until the test opens the gate. The copy is in progress while the test works (a removal of its folder, say).
    PauseCopy {
        /// The file whose copy is held.
        file: FileId,
        /// The gate the job waits at: a test's own static, so each test has its own.
        gate: &'static PauseGate,
    },
}

/// A gate a test holds a job at (`Fault::PauseCopy`). The job's [`PauseGate::enter`] waits until the test opens the gate,
/// and [`PauseGate::entered`] tells the test that the job has reached it. `const` to make, so a test keeps its gate in a
/// `static`.
pub struct PauseGate {
    /// (the job has reached the gate, the test has opened it).
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl PauseGate {
    /// A gate nobody has reached, not open.
    pub const fn new() -> PauseGate {
        PauseGate { state: Mutex::new((false, false)), changed: Condvar::new() }
    }

    /// Says the job has reached the gate, and waits until the test opens it.
    pub fn enter(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Whether a job has reached the gate.
    pub fn entered(&self) -> bool {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).0
    }

    /// Lets the held job go on.
    pub fn open(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).1 = true;
        self.changed.notify_all();
    }
}

impl Default for PauseGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Gates are the same gate when they are the same static: `Fault` compares and prints by identity.
impl PartialEq for PauseGate {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for PauseGate {}

impl std::fmt::Debug for PauseGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PauseGate")
    }
}

/// The failure of [`Fault::FailCopy`], at the start of a copy into the original. [`Fault::PauseCopy`] holds the copy here,
/// once its temp is recorded (every entry after the gate is open passes).
pub(crate) fn in_copy(fault: Option<Fault>, file: FileId) -> std::io::Result<()> {
    if let Some(Fault::PauseCopy { file: held, gate }) = fault
        && held == file
    {
        gate.enter();
    }
    if fault == Some(Fault::FailCopy(file)) {
        return Err(std::io::Error::other("a fault: the copy fails"));
    }
    Ok(())
}

#[expect(clippy::panic, reason = "the fault asked for is a panic")]
pub(crate) fn in_process(fault: Option<Fault>, file: FileId) {
    if fault == Some(Fault::PanicInProcess(file)) {
        panic!("a fault: a panic in the job of file {}", file.get());
    }
}

#[expect(clippy::panic, reason = "the fault asked for is a panic")]
pub(crate) fn in_drain(fault: Option<Fault>) {
    if fault == Some(Fault::PanicInDrain) {
        panic!("a fault: a panic in the drain");
    }
}

/// The save of [`Fault::SaveBeforeCopy`], the first time the job of its file reaches the copy (`fired` says
/// whether it did).
pub(crate) fn before_copy(fault: Option<Fault>, fired: &AtomicBool, file: FileId, path: &Path) -> std::io::Result<()> {
    if let Some(Fault::SaveBeforeCopy { file: saved, with }) = fault
        && saved == file
        && !fired.swap(true, Ordering::SeqCst)
    {
        std::fs::write(path, with)?;
    }
    Ok(())
}

/// The row removal of [`Fault::ForgetBeforeRecord`], the first time the job of its file reaches the record: the job's
/// row is removed through a catalog of its own, as a change that cancels the write would do.
pub(crate) fn before_record(
    fault: Option<Fault>,
    fired: &AtomicBool,
    job: &pg_catalog::queue::Job,
    catalog_path: &Path,
) -> crate::error::Result<()> {
    if let Some(Fault::ForgetBeforeRecord(forgotten)) = fault
        && forgotten == job.file_id
        && !fired.swap(true, Ordering::SeqCst)
    {
        let catalog = pg_catalog::Catalog::open(catalog_path, pg_platform::native::PATH_RULES)?;
        catalog.drop_gone_write(job)?;
    }
    Ok(())
}
