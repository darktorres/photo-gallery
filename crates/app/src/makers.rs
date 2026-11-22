//! The thumbnail makers: each one a decoder with its buffers. The image work borrows one, and an idle one is kept for the next
//! borrower, up to `KEEP_MAKERS` of them.

use pg_render::maker::Maker;

use crate::error::{Result, failed};
use crate::pool::Pool;

/// Idle thumbnail makers kept (each holds at most `pg_render::maker::KEEP_BYTES` of buffers).
const KEEP_MAKERS: usize = 8;

/// The thumbnail makers of the app.
pub(crate) struct Makers {
    pool: Pool<Maker>,
}

impl Makers {
    pub(crate) fn new() -> Makers {
        Makers { pool: Pool::new(KEEP_MAKERS) }
    }

    /// Run `f` with a thumbnail maker (and its decoder): an idle one, or a new one. What `f` returns is its own: an error
    /// of the work is the caller's to say, and only a maker that can't be made is an error here.
    pub(crate) fn with_maker<R>(&self, f: impl FnOnce(&mut Maker) -> R) -> Result<R> {
        self.pool.with(|| Maker::new().map_err(failed), |m| Ok(f(m)))
    }
}
