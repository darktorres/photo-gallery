//! Faults the tests put in the writer ([`Writer::start_with_fault`](crate::Writer::start_with_fault)), with the
//! `fault` feature (or in this crate's own tests).

use pg_model::ids::FileId;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the writer fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// In a job of this file, as a damaged file the edit can't take would: the job fails, and the writer goes on.
    PanicInProcess(FileId),
    /// In the drain, outside any job (where it reads the queue): the writer thread ends.
    PanicInDrain,
    /// Once, in a job of `file`: another program saves the file as `with`, after the writer read it and before it
    /// opens it for the copy.
    SaveBeforeCopy { file: FileId, with: &'static [u8] },
    /// Once, in a job of `file`: the job's row leaves the queue after the writer read it and opened the original for
    /// the copy, before the temp is recorded (a change that cancels the write does that, as a rating set back to what
    /// the file holds does).
    ForgetBeforeRecord(FileId),
    /// Every copy of this file fails once its temp is recorded (the copy into the original fails, as with a disk
    /// full), for a fresh job and a finishing copy alike: the copy is tried up to the cap.
    FailCopy(FileId),
}

/// The failure of [`Fault::FailCopy`], at the start of a copy into the original.
pub(crate) fn in_copy(fault: Option<Fault>, file: FileId) -> std::io::Result<()> {
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
pub(crate) fn before_copy(
    fault: Option<Fault>,
    fired: &AtomicBool,
    file: FileId,
    path: &Path,
) -> std::io::Result<()> {
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
    job: &pg_catalog::writes::Job,
    catalog_path: &Path,
) -> anyhow::Result<()> {
    if let Some(Fault::ForgetBeforeRecord(forgotten)) = fault
        && forgotten == job.file_id
        && !fired.swap(true, Ordering::SeqCst)
    {
        let catalog = pg_catalog::Catalog::open(catalog_path, pg_platform::native::PATH_RULES)?;
        pg_catalog::writes::drop_gone(&catalog, job)?;
    }
    Ok(())
}
