//! Where a search or a save gets its Yandex calls. A leaf: its signatures use only `pg_yandex` types, so the parent
//! (`yandex`) and its child `replay` (which implements the trait for a recorded session) both depend on it, and it depends
//! on neither.

use pg_yandex::{OtherSize, Similar, Upload, YandexError};

/// Where a search or a save gets its Yandex calls: the real client, or a recorded session that stands in for it.
/// Each search and each save makes its own, on its thread.
pub trait Source {
    fn upload(&mut self, image: &[u8]) -> Result<Upload, YandexError>;
    fn other_sizes(&mut self, up: &Upload) -> Result<Vec<OtherSize>, YandexError>;
    fn similar(&mut self, up: &Upload, max_pages: u32) -> Result<Similar, YandexError>;
}
