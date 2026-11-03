//! What the UI browses (PLAN §7): a folder tree where folders with the same path on different drives are one
//! node ("merged drives"), the files of a node filtered and sorted, ratings and turns (queued for the files),
//! saved views and where the user was.

pub mod read;
pub mod writes;

#[cfg(test)]
mod fixtures;
