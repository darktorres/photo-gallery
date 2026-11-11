//! The filter bar's counts: how many of a folder's files match each value of each facet.

use std::collections::BTreeMap;

use crate::core::Catalog;
use crate::error::Result;
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use strum::{EnumCount, IntoEnumIterator};

use super::grid::FILE_JOINS;
use super::scope::scope;
use pg_model::media::MediaType;
use pg_model::query::{BorderFacet, Query};
use pg_model::rules::Merged;
use pg_query::{Counted, MEDIA_CASE, border_counts, condition_without};

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
    /// Per border value (`BorderFacet::iter()`'s order), the same way.
    pub borders: Vec<(BorderFacet, i64)>,
}

impl Catalog {
    /// The facets, from one pass over the folder's files. Each count is the query without the facet that count is for
    /// (`condition_without`), so a file is counted in every condition it passes: the pass sums each condition per
    /// (type, extension), and the counts are sums of those. One scan where five statements read the folder's rows, and
    /// the joins, five times.
    pub fn facets(&self, folder: &Merged, recursive: bool, query: &Query) -> Result<Facets> {
        let conn = &self.conn;
        let (scope, scope_values) = scope(self.rules(), folder, recursive);
        let all = condition_without(query, None);
        let types = condition_without(query, Some(Counted::Types));
        let exts = condition_without(query, Some(Counted::Exts));
        let borders = condition_without(query, Some(Counted::Borders));

        // The select list, in order: the group's type and extension, the files, the matching files, the files of each
        // type and of each extension, and of each border value. Each condition's values are bound where it appears.
        let mut select = vec![MEDIA_CASE.to_string(), "f.ext".to_string(), "count(*)".to_string()];
        let mut values: Vec<Value> = Vec::new();
        for cond in [&all, &types, &exts] {
            select.push(format!("coalesce(sum(CASE WHEN ({}) THEN 1 ELSE 0 END), 0)", cond.sql));
            values.extend(cond.params.iter().cloned());
        }
        let border_terms = border_counts(&borders.sql);
        for term in &border_terms {
            select.push(term.clone());
            values.extend(borders.params.iter().cloned());
        }
        values.extend(scope_values);
        let sql = format!(
            "SELECT {} FROM files f {FILE_JOINS} WHERE f.missing = 0 AND {scope} GROUP BY {MEDIA_CASE}, f.ext",
            select.join(", ")
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values), |r| {
            let mut sums = [0_i64; 3 + BorderFacet::COUNT];
            for (i, sum) in sums.iter_mut().enumerate() {
                *sum = r.get(3 + i)?;
            }
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, sums))
        })?;

        let (mut total, mut matching) = (0, 0);
        let mut by_type: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_ext: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_border = [0_i64; BorderFacet::COUNT];
        for row in rows {
            let (media, ext, n, sums) = row?;
            let [all_n, types_n, exts_n] = [sums[0], sums[1], sums[2]];
            total += n;
            matching += all_n;
            *by_type.entry(media).or_default() += types_n;
            *by_ext.entry(ext).or_default() += exts_n;
            for (sum, border) in by_border.iter_mut().zip(&sums[3..]) {
                *sum += border;
            }
        }

        // Only the values some file has, as the grouped statements listed them.
        let mut types: Vec<(MediaType, i64)> = by_type
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .filter_map(|(t, n)| Some((MediaType::from_key(&t)?, n)))
            .collect();
        types.sort();
        let mut exts: Vec<(String, i64)> = by_ext.into_iter().filter(|(_, n)| *n > 0).collect();
        exts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let borders = BorderFacet::iter().zip(by_border).collect();
        Ok(Facets { total, matching, types, exts, borders })
    }
}
