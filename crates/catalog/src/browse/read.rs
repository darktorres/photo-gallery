//! The read side of browsing (PLAN §7): the folder tree (merged across drives), the files of a node filtered and
//! sorted, and what the viewer and the thumbnails need of one file, each a method of `Catalog` in its module. Nothing
//! here writes; the saved views and the session are `browse::views`'.

mod changes;
mod details;
pub mod facets;
pub(crate) mod grid;
mod scope;
mod state;
mod tree;

#[cfg(test)]
mod tests;
