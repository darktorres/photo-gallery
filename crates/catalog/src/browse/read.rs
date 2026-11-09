//! The read side of browsing (PLAN §7): the folder tree (merged across drives), the files of a node filtered and
//! sorted, what the viewer and the thumbnails need of one file, and the views and the session as they were saved.
//! Nothing here writes; the commands are in [`super::writes`].

mod changes;
mod details;
mod facets;
mod grid;
mod scope;
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

#[cfg(test)]
mod tests;
