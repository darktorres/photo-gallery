//! The points of the protocol where a test can run something, and the hook that runs there (see the parent module). The
//! hook is for tests only: a build without `cfg(test)` only reads the step's path.

use std::path::Path;

/// A point in the protocol where a test can run something: the races the tests cover happen there.
#[derive(Clone, Copy)]
pub(crate) enum Step<'a> {
    /// A claim has made its hidden name (Unix), before it is locked.
    #[cfg(unix)]
    ClaimHidden(&'a Path),
    /// A claim holds its lock, before it makes its folder.
    ClaimLocked(&'a Path),
    /// A claim has made its folder.
    ClaimMade(&'a Path),
    /// A cleanup has found a folder, before it takes the folder's lock.
    SweepBeforeAcquire(&'a Path),
    /// A cleanup has taken an earlier build's folder's owner file, and not yet removed the folder.
    OldBeforeRemove(&'a Path),
    /// A lock is taken (Unix), before its path is checked to name the file that was locked.
    #[cfg(unix)]
    TakeLocked(&'a Path),
    /// A lock's name is gone (Unix), before its file is closed.
    #[cfg(unix)]
    BeforeClose(&'a Path),
}

impl<'a> Step<'a> {
    fn path(self) -> &'a Path {
        match self {
            #[cfg(unix)]
            Step::ClaimHidden(path) => path,
            Step::ClaimLocked(path) | Step::ClaimMade(path) | Step::SweepBeforeAcquire(path) => path,
            Step::OldBeforeRemove(path) => path,
            #[cfg(unix)]
            Step::TakeLocked(path) => path,
            #[cfg(unix)]
            Step::BeforeClose(path) => path,
        }
    }
}

#[cfg(test)]
pub(crate) type Hook = Box<dyn for<'a> FnMut(&Step<'a>)>;

#[cfg(test)]
thread_local! {
    static HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
}

/// Runs the test hook at `step`. The hook is taken out while it runs, so what it does (a cleanup, a claim) passes through
/// the same points without calling itself.
pub(crate) fn seam(step: Step<'_>) {
    // Read in every build, so a build without hooks uses the step's path too.
    let _ = step.path();
    #[cfg(test)]
    {
        let hook = HOOK.with(|slot| slot.borrow_mut().take());
        if let Some(mut hook) = hook {
            hook(&step);
            HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
        }
    }
}

/// A test's hook, installed for the test on this thread until the guard drops.
#[cfg(test)]
pub(crate) struct Hooked;

#[cfg(test)]
impl Drop for Hooked {
    fn drop(&mut self) {
        let _ = HOOK.with(|slot| slot.borrow_mut().take());
    }
}

/// Installs `hook` on this thread until the guard drops.
#[cfg(test)]
pub(crate) fn set(hook: Hook) -> Hooked {
    HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
    Hooked
}
