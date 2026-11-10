//! What the front end names from the crates under the app, re-exported here so that the front end depends on `pg-app`
//! alone: the job's progress and phases, the library's changes and notices, the thumbnail cache's events, the writer's
//! failures, the Yandex client's errors, the catalog's folder facets (plain data), and the two checks the front end
//! makes on the crates' behalf (the panic check, the cache's size parse). Nothing here is a call into a crate's data
//! layer: the reads and writes are on the services that [`crate::App`] opens, and [`crate::Session`].

pub use pg_catalog::browse::read::Facets;
pub use pg_jobs::control::{Phase, Progress};
pub use pg_jobs::events::Change;
pub use pg_jobs::failure::{Cause, FileError};
pub use pg_jobs::panics::caught as panic_caught;
pub use pg_library::runner::Notice;
pub use pg_thumbcache::Event as ThumbEvent;
pub use pg_thumbcache::StartOverReason as ThumbStartOverReason;
pub use pg_thumbcache::parse_cap_gb as parse_thumb_cap_gb;
pub use pg_writeback::WriteError;
pub use pg_yandex::YandexError;
