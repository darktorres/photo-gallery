//! What the user changes about a file: its rating, its border mark. Turns are in `library.rs` (they go through the writer).
//! A rating is written into the file too, unless the app's preferences say the catalog only (`write_back`).

use pg_model::ids::FileId;
use pg_model::rating::Rating;

use crate::app::App;
use crate::error::Result;

impl App {
    /// Set (or clear) a file's rating, and queue it for the file when write-back is on.
    pub fn rate(&self, file_id: FileId, rating: Option<Rating>) -> Result<()> {
        let write_back = self.prefs().write_back;
        self.with_catalog(|c| pg_catalog::writes::set_rating(c, file_id, rating, write_back))?;
        self.wake_writer();
        Ok(())
    }

    /// Mark (or unmark) a file's borders as ones to keep (the cropping job reads the mark).
    pub fn set_border_mark(&self, file_id: FileId, marked: bool) -> Result<()> {
        self.with_catalog(|c| c.set_border_mark(file_id, marked))
    }
}
