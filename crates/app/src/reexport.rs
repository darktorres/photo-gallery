//! What the front end names from the crates under the app, re-exported here so that the front end depends on `pg-app`
//! alone: the library's changes, the inbox's entries, their counts and a ledger's files and folders (with why a folder
//! failed), the writer's failures, the catalog's folder facets (plain data), and the panic check the front end makes on
//! the crates' behalf, and the cache's size limit (`ThumbCap`). Nothing here is a call into a crate's data layer: the
//! reads and writes are on the services that [`crate::App`] opens, and [`crate::Session`]. (`pg_model`'s types, such as
//! `LedgerKind`, the front end names from `pg-model`, which it depends on itself.)

pub use pg_catalog::FolderFailure;
pub use pg_catalog::browse::read::facets::Facets;
pub use pg_catalog::inbox::{Entry as InboxEntry, EntryId as InboxEntryId};
pub use pg_catalog::ledger::{Counts as InboxCounts, FailedFile, FailedFolder};
pub use pg_jobs::events::Change;
pub use pg_jobs::failure::Cause;
pub use pg_jobs::panics::caught as panic_caught;
pub use pg_thumbcache::Cap as ThumbCap;
pub use pg_writeback::WriteError;
