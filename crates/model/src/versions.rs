//! The versions of every persisted artifact, in one place. Each is an `i64` named `<OWNER>_VERSION`, and the name is the
//! upper-cased mark it drives in `meta` (`import_version` is `IMPORT_VERSION`, `thumb_output_version` is
//! `THUMB_OUTPUT_VERSION`; `KEY_ALGO_VERSION` is the one exception: it drives the `key_version` mark, and its doc says why).
//!
//! **Before the first release nothing versioned is bumped.** That covers every constant here and every other version,
//! layout name or format mark in the repository: the catalog's schema version, the bench data's layout names
//! (`BENCH_LAYOUT` in `crates/bench/benches/ir/catalog.rs`), a fixture's or a file's format number. Each stays at 1 (a
//! name at `v1`), and a change to what an artifact stores, or to what its stored values mean, is made in place. An
//! artifact an earlier build made is not told apart by its number: the catalog and the thumbnail cache notice a changed
//! schema by themselves (a rebuild, a start over), a mark that carries its inputs' identity notices those (the
//! `key_version` mark's table hash), and anything else a development build made (a catalog's analyses, bench data, a
//! dataset) is deleted and made again by hand. Docs and plans write a change as an edit at version 1, never as a bump.
//!
//! From the first release on, a bump is a change to what the artifact stores, or to what its stored values mean. Its rule
//! is written beside each constant. `tools/test_versions.py` fails on a `*VERSION` constant anywhere in the workspace that is not in this
//! module, and on a constant here that no owner reads, so a new version is registered here or it is not a version.
//! Owners name these constants as `pg_model::versions::NAME`; no crate re-exports one under its own path.

/// The analysis pipeline's version (`pg_analysis::analyze::IMPORT_VERSION`): an analysis made at another is redone. Bump
/// it when the import computes a different value from the same file: the pixel or file hash, the decode bounds, the PDQ
/// hash, the EXIF date and zone semantics.
pub const IMPORT_VERSION: i64 = 1;

/// The version of the borders pass (`pg_analysis::borders::BORDERS_VERSION`): the detection of a picture's borders.
pub const BORDERS_VERSION: i64 = 1;

/// The version of the pairs the pairing pass makes (`pg_model::pdq::PAIRING_VERSION`): what matches within
/// `MATCH_DISTANCE`, among the reliable hashes. A catalog at another owes every pair again. Its seed in the baseline is
/// this value. The seed is a literal in `baseline.sql` (SQL cannot name this constant), so the catalog's `schema.rs` asserts
/// it at compile time and its schema test checks the stored seed.
pub const PAIRING_VERSION: i64 = 1;

/// The version of the thumbnails' pass state: the retry semantics of a thumbnail the pass made. It is bumped by hand when
/// a change to the decoders or the maker makes a file that failed before decode now (a codec upgrade is not detected). A
/// catalog's or a cache's mark that differs (`pg_catalog::Versions::thumbs`, `pg_thumbcache::ThumbCache::open`) tries
/// again the files whose thumbnail panicked, and the next pass looks at every file again.
pub const THUMBS_VERSION: i64 = 1;

// Zero is the mark of a catalog or a cache that has none: this build's thumbnails must differ from it, or a bump is missed.
const _: () = assert!(THUMBS_VERSION > 0);

/// The version of a placeholder's definition (`pg_model::analysis::PLACEHOLDER_VERSION`): a placeholder made at another
/// is owed again. Bump it when the ThumbHash of a picture changes for the same pixels.
pub const PLACEHOLDER_VERSION: i64 = 1;

/// The version of the thumbnail cache's layout (`pg_thumbcache::CACHE_VERSION`). A cache at another starts over; a newer
/// one is left alone.
pub const CACHE_VERSION: i64 = 1;

/// The version of the replay format (`pg_client::controller::replay::REPLAY_FORMAT_VERSION`): a dev recording's format.
pub const REPLAY_FORMAT_VERSION: i64 = 1;

/// The version of the catalog's schema this build makes (`pg_catalog`'s current version): the baseline, then each
/// migration. The catalog checks it against its baseline and migrations (`schema.rs`).
pub const CATALOG_SCHEMA_VERSION: i64 = 1;

/// The version of the thumbnail pixels this build makes: the resize, the JPEG and
/// WebP settings, the chroma. A cache whose `meta.thumb_output_version` is another starts over; a newer one is left
/// alone. Bump it when a thumbnail's bytes change for the same source.
pub const THUMB_OUTPUT_VERSION: i64 = 1;

/// The version of the key algorithm (`pg_model::rules`'s keys: `folders.path_key`, the spelling a folder is found by;
/// `mfolders.key`, the drive-free folder key the grid orders by; and the name key of the `path_name_key` SQL function,
/// which the `files_mn` index holds). `path_key` is the folder's identity (one row per spelling in any case under the
/// Windows rules); the Windows fold is NTFS's per-unit upcase through an embedded table (`pg_model::upcase`), and the
/// Posix keys are the exact bytes. Bump it when a key changes for the same name, or when a key's meaning does. The
/// catalog's `key_version` mark is this with the table's hash (`upcase::TABLE_HASH`): either changing recomputes the
/// stored keys and rebuilds the name index.
pub const KEY_ALGO_VERSION: i64 = 1;

/// The version of the preferences file's layout (`gallery.json`, `pg_app::prefs`). Each save stamps it; a file without the
/// key reads as 0, the layout whose row height was a float. Version 1 stores the row height as a whole number of pixels.
/// Bump it when a field changes meaning or its format. A new field needs no bump: a file without it reads the default.
pub const PREFS_VERSION: i64 = 1;
