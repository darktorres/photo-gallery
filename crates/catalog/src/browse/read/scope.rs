//! The condition on a folder's files: the range of order keys of the folder, and of everything below it when `recursive`.
//! A leaf of the read side: `facets` and `grid` use it, and it uses nothing of theirs.

use rusqlite::types::Value;

use pg_model::rules::{Merged, PathRules};

/// The files of a folder (optionally with everything below it), as a range of order keys under `rules`: a condition
/// on `f` and the values it binds, which the `files_order` index answers. The range is the one the writer's keys make
/// ([`PathRules::scope_keys`]), so a folder is found by the same case rule it was stored under.
pub(super) fn scope(rules: PathRules, folder: &Merged, recursive: bool) -> (String, Vec<Value>) {
    let (lo, hi) = rules.scope_keys(folder.parts(), recursive);
    let mut values = vec![Value::Blob(lo)];
    let mut sql = "f.order_key >= ?".to_string();
    if let Some(hi) = hi {
        sql.push_str(" AND f.order_key < ?");
        values.push(Value::Blob(hi));
    }
    (sql, values)
}
