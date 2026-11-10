//! The state of the `FailedFiles` QObject: the bridge's backing struct (the files shown).

use pg_app::reexport::FailedFile;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct FailedFilesRust {
    pub(crate) total: i32,
    /// The files shown, newest first.
    pub(super) rows: Vec<FailedFile>,
}
