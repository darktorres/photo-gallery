//! Where the tests of the crates above ask the borders pass to fail (the `fault` feature): the detector panics on one
//! file, as a real detector's bug would, so a test sees the panic recorded in the failure ledger.

use std::sync::{Mutex, PoisonError};

use pg_model::ids::FileId;

/// The file whose detector panics, if any.
static PANIC_ON: Mutex<Option<FileId>> = Mutex::new(None);

/// From now on, the detector panics on `file` (`None`: on none). The fault is the process's: a test that sets it holds
/// it alone, and clears it before it ends.
pub fn panic_on(file: Option<FileId>) {
    *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner) = file;
}

/// Called before the detector looks at `file`: panics when it is the one asked for.
#[expect(clippy::panic, reason = "the fault the tests ask for: a detector that panics")]
pub(crate) fn in_detector(file: FileId) {
    let asked = *PANIC_ON.lock().unwrap_or_else(PoisonError::into_inner);
    if asked == Some(file) {
        panic!("a fault: the detector panics on file {}", file.get());
    }
}
