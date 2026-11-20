//! The versions of every persisted artifact, in one place. Each is an `i64` named `<OWNER>_VERSION`, and the name is the
//! upper-cased mark it drives in `meta` (`import_version` is `IMPORT_VERSION`, `thumb_output_version` is
//! `THUMB_OUTPUT_VERSION`). The owning crate re-exports its constant under the same name with a pointer to this module:
//! `pub const IMPORT_VERSION: i64 = pg_model::versions::IMPORT_VERSION;`.
//!
//! A bump is a change to what the artifact stores, or to what its stored values mean. Its rule is written beside each
//! constant. `tools/test_versions.py` fails on a `*VERSION` constant anywhere in the workspace that is not in this
//! module, and on a constant here that no owner reads, so a new version is registered here or it is not a version.

/// The analysis pipeline's version (`pg_analysis::analyze::IMPORT_VERSION`): an analysis made at another is redone. Bump
/// it when the import computes a different value from the same file: the pixel or file hash, the decode bounds, the PDQ
/// hash, the EXIF date and zone semantics.
pub const IMPORT_VERSION: i64 = 1;

/// The version of the borders pass (`pg_analysis::borders::BORDERS_VERSION`): the detection of a picture's borders.
pub const BORDERS_VERSION: i64 = 1;

/// The version of the pairs the pairing pass makes (`pg_model::pdq::PAIRING_VERSION`): what matches within
/// `MATCH_DISTANCE`, among the reliable hashes. A catalog at another owes every pair again. Its seed in the baseline is
/// this value, checked by the catalog's schema test.
pub const PAIRING_VERSION: i64 = 1;

/// The version of the thumbnails' pass state (`pg_render::THUMBS_VERSION`): the retry semantics of a thumbnail the pass
/// made. Bump it when the pass must look at every file again.
pub const THUMBS_VERSION: i64 = 1;

/// The version of a placeholder's definition (`pg_model::analysis::PLACEHOLDER_VERSION`): a placeholder made at another
/// is owed again. Bump it when the ThumbHash of a picture changes for the same pixels.
pub const PLACEHOLDER_VERSION: i64 = 1;

/// The version of the thumbnail cache's layout (`pg_thumbcache::CACHE_VERSION`). A cache at another starts over; a newer
/// one is left alone (plan D9).
pub const CACHE_VERSION: i64 = 1;

/// The version of the replay format (`pg_client::controller::replay::REPLAY_FORMAT_VERSION`): a dev recording's format.
pub const REPLAY_FORMAT_VERSION: i64 = 1;

/// The version of the catalog's schema this build makes (`pg_catalog`'s current version): the baseline, then each
/// migration. The catalog checks it against its baseline and migrations (`schema.rs`).
pub const CATALOG_SCHEMA_VERSION: i64 = 1;

/// The version of the thumbnail pixels this build makes (`pg_render::THUMB_OUTPUT_VERSION`): the resize, the JPEG and
/// WebP settings, the chroma. A cache whose `meta.thumb_output_version` is another starts over; a newer one is left
/// alone (plan D9). Bump it when a thumbnail's bytes change for the same source.
pub const THUMB_OUTPUT_VERSION: i64 = 1;

/// The version of the key algorithm (`pg_model::name`'s folder and order keys: `folders.key`, `files.order_key`). Bump it
/// when a key changes for the same name. The catalog's `key_version` mark is this with the toolchain's Unicode version,
/// which the keys' case folding depends on: either changing recomputes the stored keys.
pub const KEY_ALGO_VERSION: i64 = 1;
