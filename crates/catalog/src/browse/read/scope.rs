//! The condition on a folder's files: the range of merged folder keys of the folder, and of everything below it when
//! `recursive`. A leaf of the read side: `facets` and `grid` use it, and it uses nothing of theirs.

use rusqlite::types::Value;

use pg_model::rules::{Merged, PathRules};

/// What a statement over a scope's files reads them from: the merged folders (`mf`, the scope's range) outside, each
/// one's files (`f`) by the name index inside (`files_mn`: a merged folder's files, present and missing apart, in name-key
/// order). `CROSS JOIN` pins the order of the loops; `INDEXED BY` pins the index, which the planner, with no statistics,
/// otherwise misjudges.
pub(super) const SCOPE_FROM: &str = "mfolders mf CROSS JOIN files f INDEXED BY files_mn ON f.mid = mf.id";

/// The files of a folder (optionally with everything below it), as a range of merged folder keys under `rules`: a
/// condition on `mf` ([`SCOPE_FROM`]) and the values it binds. The range is the one the writer's keys make
/// ([`PathRules::scope_keys`]), so a folder is found by the same case rule it was stored under.
pub(super) fn scope(rules: PathRules, folder: &Merged, recursive: bool) -> (String, Vec<Value>) {
    let (lo, hi) = rules.scope_keys(folder.parts(), recursive);
    let mut values = vec![Value::Blob(lo)];
    let mut sql = "mf.key >= ?".to_string();
    if let Some(hi) = hi {
        sql.push_str(" AND mf.key < ?");
        values.push(Value::Blob(hi));
    }
    (sql, values)
}
