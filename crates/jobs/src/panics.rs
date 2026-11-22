//! Panics on a job's thread. A panic a job catches is reported by the job (its status line, the log), not as a box
//! on a GUI program: the process's panic hook asks [`caught`] before it shows one.
//!
//! [`catch`] is for a whole unit of work (a round, a read the UI asked for); [`guard`] is for one file of a pass, whose
//! panic becomes that file's error ([`Panicked`](crate::failure::Panicked)) and the pass goes on with the next one.

use std::any::Any;
use std::cell::Cell;
use std::panic::UnwindSafe;

use crate::failure::Failed;

thread_local! {
    /// Set while [`catch`] runs its closure on this thread.
    static CAUGHT: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, catching a panic (see the module doc). The caller must report the error it returns.
///
/// It nests: a `catch` inside another (a file's [`guard`] inside a caught round) leaves the flag as the outer one set
/// it, so a panic after the inner one returns still counts as caught.
pub fn catch<T>(f: impl FnOnce() -> T + UnwindSafe) -> std::thread::Result<T> {
    let outer = CAUGHT.with(|c| c.replace(true));
    let result = std::panic::catch_unwind(f);
    CAUGHT.with(|c| c.set(outer));
    result
}

/// Whether a panic starting on this thread is inside [`catch`].
pub fn caught() -> bool {
    CAUGHT.with(Cell::get)
}

/// The message a panic carried, for the report.
pub fn message(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "no message"
    }
}

/// Runs `f` on one item of work, a panic in it caught (through [`catch`], so the panic hook only logs it) and returned
/// as a [`Failed`] of class [`crate::failure::Class::Panic`]. An error of `f` becomes a `Failed` through its `From`.
pub fn guard<T, E>(f: impl FnOnce() -> Result<T, E> + UnwindSafe) -> Result<T, Failed>
where
    E: Into<Failed>,
{
    match catch(f) {
        Ok(result) => result.map_err(Into::into),
        Err(payload) => Err(Failed::panic(message(&*payload).to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::Class;

    #[test]
    fn a_guarded_panic_is_its_message_and_leaves_the_outer_catch_in_place() {
        let outer = catch(|| {
            let inner = guard(|| -> Result<(), Failed> { panic!("the file fails") });
            let failed = inner.unwrap_err();
            assert_eq!(failed.class(), Class::Panic);
            assert_eq!(failed.text(), "the file fails");
            // Still inside the outer catch: a panic now is caught too.
            caught()
        });
        assert!(outer.unwrap());
        assert!(!caught());
    }

    #[test]
    fn a_guarded_error_keeps_its_class() {
        let failed = guard(|| -> Result<(), Failed> { Err(Failed::new(Class::Data, ReadTest)) }).unwrap_err();
        assert_eq!((failed.class(), failed.is_panic()), (Class::Data, false));
        assert_eq!(guard(|| Ok::<u8, Failed>(3)).unwrap(), 3);
    }

    #[derive(Debug)]
    struct ReadTest;

    impl std::fmt::Display for ReadTest {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("no")
        }
    }

    impl std::error::Error for ReadTest {}
}
