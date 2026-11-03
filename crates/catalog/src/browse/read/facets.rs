//! The filter bar's counts: how many of a folder's files match each value of each facet.

use anyhow::Result;
use rusqlite::{Connection, params_from_iter};

use super::grid::FILE_JOINS;
use super::scope;
use crate::query::{BORDER_SUMS, BorderFacet, Facet, MediaType, Query};

/// What the filter bar shows next to its choices, for a view's folder.
#[derive(Default, Debug, PartialEq)]
pub struct Facets {
    /// Present files in the folder, unfiltered.
    pub total: i64,
    /// Those the query passes.
    pub matching: i64,
    /// Per type and per extension (most files first): how many files choosing it would show, given the other
    /// facets.
    pub types: Vec<(MediaType, i64)>,
    pub exts: Vec<(String, i64)>,
    /// Per border value (`BorderFacet::ALL`'s order), the same way.
    pub borders: Vec<(BorderFacet, i64)>,
}

pub fn facets(conn: &Connection, merged: &str, recursive: bool, query: &Query) -> Result<Facets> {
    let (scope, scope_values) = scope(merged, recursive);
    let count = |skip: Option<Facet>, select: &str, group: &str| -> Result<Vec<(String, i64)>> {
        let cond = query.condition_without(skip);
        let sql = format!(
            "SELECT {select}, count(*) FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {} {group}",
            cond.sql
        );
        let values = scope_values.iter().cloned().chain(cond.params);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
        params_from_iter(&scope_values),
        |r| r.get(0),
    )?;
    let matching = count(None, "''", "")?.first().map_or(0, |r| r.1);
    let mut types: Vec<(MediaType, i64)> = count(Some(Facet::Types), MediaType::CASE, "GROUP BY 1")?
        .into_iter()
        .filter_map(|(t, n)| Some((MediaType::from_key(&t)?, n)))
        .collect();
    types.sort();
    let exts = count(Some(Facet::Exts), "f.ext", "GROUP BY 1 ORDER BY 2 DESC, 1")?;
    // In one pass: each value's condition summed (a file can have sides of two classes).
    let cond = query.condition_without(Some(Facet::Borders));
    let sql =
        format!("SELECT {BORDER_SUMS} FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {}", cond.sql);
    let values = scope_values.iter().cloned().chain(cond.params);
    let borders = conn.query_row(&sql, params_from_iter(values), |r| {
        BorderFacet::ALL.iter().enumerate().map(|(i, &b)| Ok((b, r.get(i)?))).collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(Facets { total, matching, types, exts, borders })
}
