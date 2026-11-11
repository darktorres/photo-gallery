//! What the front end names from the crates under the app, re-exported here so that the front end depends on `pg-app`
//! alone: the library's changes, the inbox's entries and a failure ledger's files, the writer's failures, the catalog's
//! folder facets (plain data), and the two checks the front end makes on the crates' behalf (the panic check, the cache's size parse). Nothing here is a call into a crate's data
//! layer: the reads and writes are on the services that [`crate::App`] opens, and [`crate::Session`].

pub use pg_catalog::browse::read::facets::Facets;
pub use pg_catalog::inbox::{Entry as InboxEntry, EntryId as InboxEntryId};
pub use pg_catalog::ledger::FailedFile;
pub use pg_jobs::events::Change;
pub use pg_jobs::failure::{Cause, FileError};
pub use pg_jobs::panics::caught as panic_caught;
pub use pg_thumbcache::parse_cap_gb as parse_thumb_cap_gb;
pub use pg_writeback::WriteError;
