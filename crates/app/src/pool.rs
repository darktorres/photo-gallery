//! Bounded pools of what the image and read work borrows: catalog connections and thumbnail makers. A borrower takes an
//! idle one, or makes one, and gives it back when it is done; at most `keep` idle ones are kept, and the rest are
//! dropped. A pool serves whichever thread asks, so a thread holds nothing of its own between requests.

use std::sync::{Mutex, PoisonError};

pub struct Pool<T> {
    idle: Mutex<Vec<T>>,
    keep: usize,
}

impl<T> Pool<T> {
    pub fn new(keep: usize) -> Pool<T> {
        Pool { idle: Mutex::new(Vec::new()), keep }
    }

    /// Run `f` with an idle item, or with one `make` makes, and give the item back after.
    pub fn with<E, R>(
        &self,
        make: impl FnOnce() -> Result<T, E>,
        f: impl FnOnce(&mut T) -> Result<R, E>,
    ) -> Result<R, E> {
        let taken = self.idle.lock().unwrap_or_else(PoisonError::into_inner).pop();
        let mut item = match taken {
            Some(item) => item,
            None => make()?,
        };
        let result = f(&mut item);
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < self.keep {
            idle.push(item);
        }
        result
    }

    /// Take every idle item out, to close them at shutdown.
    pub fn drain(&self) -> Vec<T> {
        std::mem::take(&mut *self.idle.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
