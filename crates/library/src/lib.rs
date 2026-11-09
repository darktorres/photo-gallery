//! The job (`runner`) that runs the passes over the originals in order, for the app and for the command line (`pg_cli`),
//! and the passes that are the library's own: the cropped copies (`crop`), the borders (`borders`) and the duplicate
//! report (`dups`). The other passes are their own crates, which the caller depends on directly: `pg-scan`, `pg-import`,
//! `pg-thumbs` and `pg-writeback`. See PLAN §4, §8 M1.
//!
//! The passes report what they did as values (their stats, the failures they hit), and print nothing: whoever runs
//! them decides what to show, and in what words.

pub mod borders;
pub mod crop;
pub mod dups;
pub mod layout;
pub mod runner;

/// The versions of the analyses this build makes, for `Catalog::prepare`: the library passes them when it prepares a
/// catalog for writing.
pub fn versions() -> pg_catalog::Versions {
    pg_catalog::Versions {
        import: pg_analysis::analyze::IMPORT_VERSION,
        borders: pg_analysis::borders::BORDERS_VERSION,
    }
}
