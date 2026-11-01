//! Passes over the originals (scan, import, borders, thumbnails) and the reports on them, shared by the app's
//! background job (`runner`) and its command line (`cli`). See PLAN §4, §8 M1.

pub mod borders;
pub mod cli;
pub mod control;
pub mod crop;
pub mod dups;
pub mod import;
pub mod pipeline;
pub mod runner;
pub mod scan;
pub mod thumbs;
pub mod writeback;
