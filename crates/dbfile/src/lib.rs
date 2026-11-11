//! A SQLite database file's lifecycle, shared by the catalog and the thumbnail cache, each of which keeps its own policy
//! (the catalog migrates, rebuilds and carries its rows; the cache starts over):
//! - [`schema`]: the shape of a file, compared with the shape a build creates ([`Schema`], [`canonical`], [`Difference`]);
//! - [`judge`]: what a file is before anything is written to it ([`judge()`], [`Found`], [`Damage`], [`Expected`]);
//! - [`lock`]: the lock that orders the processes changing a file's layout, and the holds on an open file ([`Lock`],
//!   [`Live`]);
//! - [`remove`]: moving a file aside and deleting it, with its sidecars ([`move_aside`], [`remove_database`]).

pub mod judge;
pub mod lock;
pub mod remove;
pub mod schema;

pub use judge::{Damage, Expected, Found, judge};
pub use lock::{Live, Lock};
pub use remove::{move_aside, remove_database};
pub use schema::{Difference, Schema, application_id, canonical, user_version};
