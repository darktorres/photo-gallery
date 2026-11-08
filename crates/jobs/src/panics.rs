//! Panics on a job's thread. A panic a job catches is reported by the job (its status line, the log), not as a box
//! on a GUI program: the process's panic hook asks [`caught`] before it shows one.
//!
//! [`catch`] is for a whole unit of work (a round, a read the UI asked for); [`guard`] is for one file of a pass, whose
//! panic becomes that file's error ([`Panicked`]) and the pass goes on with the next one.

use std::any::Any;
use std::cell::Cell;
use std::panic::UnwindSafe;

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

/// A panic that [`guard`] caught: the message it carried. Its text is that message alone; the words that say it was a
/// panic are the app's (`pg_client::text`), which tells it apart from other errors by its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panicked(pub String);

impl std::fmt::Display for Panicked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Panicked {}

/// Runs `f` on one item of work, a panic in it caught (through [`catch`], so the panic hook only logs it) and returned
/// as a [`Panicked`] error.
pub fn guard<T>(f: impl FnOnce() -> anyhow::Result<T> + UnwindSafe) -> anyhow::Result<T> {
    match catch(f) {
        Ok(result) => result,
        Err(payload) => Err(anyhow::Error::new(Panicked(message(&*payload).to_string()))),
    }
}

/// Whether `error` is (or was caused by) a panic that [`guard`] caught.
pub fn is_panic(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Panicked>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guarded_panic_is_its_message_and_leaves_the_outer_catch_in_place() {
        let outer = catch(|| {
            let inner = guard(|| -> anyhow::Result<()> { panic!("the file fails") });
            let error = inner.unwrap_err();
            assert!(is_panic(&error));
            assert_eq!(error.to_string(), "the file fails");
            // Still inside the outer catch: a panic now is caught too.
            caught()
        });
        assert!(outer.unwrap());
        assert!(!caught());
    }

    #[test]
    fn a_guarded_error_is_not_a_panic() {
        let error = guard(|| -> anyhow::Result<()> { anyhow::bail!("no") }).unwrap_err();
        assert!(!is_panic(&error));
        assert_eq!(guard(|| Ok(3)).unwrap(), 3);
    }
}
