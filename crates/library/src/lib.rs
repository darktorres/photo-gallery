//! The job (`runner`) that runs the passes over the originals in order, for the app and for the command line (`pg_cli`),
//! where the data folders are (`layout`), and how a catalog folder is opened for them (`open_catalog`, `open_thumbs`,
//! `versions`). What both front ends do around the passes is here too, once: adding a folder (`folders`) and what
//! follows a scan (`scans`). The passes are their own crates, which the caller depends on directly: `pg-scan`, `pg-import`,
//! `pg-thumbs`, `pg-borders`, `pg-crop` and `pg-writeback`. See PLAN §4, §8 M1.
//!
//! The passes report what they did as values (their stats, the failures they hit), and print nothing: whoever runs
//! them decides what to show, and in what words.

pub mod folders;
pub mod layout;
pub mod runner;
pub mod scans;

/// The path of the catalog file in the catalog folder `dir`.
pub fn catalog_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("catalog.sqlite")
}

/// The catalog in the folder `dir`, prepared for this build (`Catalog::prepare` with [`versions`], under the native rules).
/// A catalog this build can't use is moved aside and rebuilt; [`pg_catalog::Catalog::rebuilt`] says so.
pub fn open_catalog(dir: &std::path::Path) -> pg_catalog::Result<pg_catalog::Catalog> {
    pg_catalog::Catalog::prepare(&catalog_path(dir), versions(), pg_platform::native::PATH_RULES)
}

/// The thumbnail cache of the catalog `catalog_id`, in the folder `dir`, holding at most `cap` bytes (`None`: the cache's
/// own cap), for this build's thumbnails (`pg_render::THUMBS_VERSION`). Its events go to `events`.
pub fn open_thumbs(
    dir: &std::path::Path,
    catalog_id: &str,
    cap: Option<i64>,
    events: std::sync::Arc<pg_jobs::events::Bus<pg_thumbcache::Event>>,
) -> anyhow::Result<pg_thumbcache::ThumbCache> {
    pg_thumbcache::ThumbCache::open(dir, catalog_id, cap, pg_render::THUMBS_VERSION, events)
}

/// The versions of the analyses this build makes, for `Catalog::prepare`: the library passes them when it prepares a
/// catalog for writing.
pub fn versions() -> pg_catalog::Versions {
    pg_catalog::Versions {
        import: pg_analysis::analyze::IMPORT_VERSION,
        borders: pg_analysis::borders::BORDERS_VERSION,
        thumbs: pg_render::THUMBS_VERSION,
    }
}
