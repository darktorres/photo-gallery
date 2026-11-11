//! Where a background worker's notices go (the app's inbox, the command line's stderr, a test's list): the job's, the
//! writer's. Here, in the job layer, so the writer (`pg-writeback`) and the job (`pg-library`) both post through it
//! without either depending on the other or on the app.

use pg_model::notice::{KeyBuf, Notice};

/// Where a worker's notices go: what it ran into, how a job asked for went, and which of its kept notices no longer
/// hold. Called on the worker's thread: an implementation queues, it doesn't wait (it takes no lock a caller may hold
/// while it posts: the writer posts holding its slot's lock, so an inbox takes only its own work lock).
pub trait NoticeSink: Send + Sync {
    /// Tell the user `notice`.
    fn post(&self, notice: Notice);
    /// The kept notices of `scope`'s kind and folder (and part, if it has one) are now `notices`: the others no longer
    /// hold (`pg_catalog::Catalog::replace_notices`).
    fn replace(&self, scope: KeyBuf, notices: Vec<Notice>);
    /// The kept notices `key` names no longer hold (a key with no part names every part).
    fn clear(&self, key: KeyBuf) {
        self.replace(key, Vec::new());
    }
    /// Something the inbox reads besides the notices changed (a pass wrote the failure ledger): read it again. Pokes
    /// coalesce; a sink with nothing to read again does nothing.
    fn poke(&self);
}
