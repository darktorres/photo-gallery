//! The read side of browsing (PLAN §7): the folder tree (merged across drives), the files of a node filtered and
//! sorted, what the viewer and the thumbnails need of one file, and the views and the session as they were saved.
//! Nothing here writes; the commands are in [`super::writes`].

use rusqlite::types::Value;

use pg_model::rules::{Merged, PathRules};

mod changes;
mod details;
mod facets;
mod grid;
mod state;
mod tree;
mod views;

pub use changes::*;
pub use details::*;
pub use facets::*;
pub use grid::*;
pub use state::*;
pub use tree::*;
pub use views::*;

/// The files of a folder (optionally with everything below it), as a range of order keys under `rules`: a condition
/// on `f` and the values it binds, which the `files_order` index answers. The range is the one the writer's keys make
/// ([`PathRules::scope_keys`]), so a folder is found by the same case rule it was stored under.
fn scope(rules: PathRules, folder: &Merged, recursive: bool) -> (String, Vec<Value>) {
    let (lo, hi) = rules.scope_keys(folder.parts(), recursive);
    let mut values = vec![Value::Blob(lo)];
    let mut sql = "f.order_key >= ?".to_string();
    if let Some(hi) = hi {
        sql.push_str(" AND f.order_key < ?");
        values.push(Value::Blob(hi));
    }
    (sql, values)
}

#[cfg(test)]
mod tests;
