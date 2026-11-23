//! The filter bar's counts: how many of a folder's files match each value of each facet.

use alloc::string::String;
use alloc::vec::Vec;

use crate::media::MediaType;
use crate::query::BorderFacet;

/// What the filter bar shows next to its choices, for a view's folder.
#[derive(Default, Debug, PartialEq)]
pub struct Facets {
    /// Present files in the folder, unfiltered.
    pub total: i64,
    /// Those the query passes.
    pub matching: i64,
    /// Per type (in `MediaType` order) and per extension (most files first): how many files choosing it would show,
    /// given the other facets.
    pub types: Vec<(MediaType, i64)>,
    /// Per extension, as (extension, count), the same way.
    pub exts: Vec<(String, i64)>,
    /// Per border value (`BorderFacet::iter()`'s order), the same way.
    pub borders: Vec<(BorderFacet, i64)>,
}
