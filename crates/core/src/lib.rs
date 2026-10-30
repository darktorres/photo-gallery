//! The library core: catalog, decoding, analysis (tiers 0–1) and thumbnails. No UI dependency (PLAN §7).

pub mod analyze;
pub mod browse;
pub mod catalog;
pub mod decode;
pub mod layout;
pub mod metadata;
pub mod pdq;
pub mod pdq_index;
pub mod resize;
pub mod rotate;
pub mod thumbs;
