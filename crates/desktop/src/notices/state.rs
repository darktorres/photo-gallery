//! The state of the `Notices` QObject: the bridge's backing struct (the entries shown, the badge, the passes linked and
//! the toast).

use cxx_qt_lib::QString;
use pg_app::reexport::InboxEntry;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct NoticesRust {
    pub(crate) unread: i32,
    pub(crate) passes: i32,
    pub(crate) toast: QString,
    pub(crate) toast_level: i32,
    /// The entries shown, newest first, as the inbox last listed them (less those dismissed since), with the marks made
    /// here since.
    pub(super) rows: Vec<InboxEntry>,
    /// `start` ran: the inbox is followed.
    pub(super) started: bool,
    /// The inbox panel is open: the inbox reads the ledgers' counts (`Inbox::set_viewing`).
    pub(super) viewing: bool,
    /// The panel is open and the window active: what is shown is read by the user, so it is marked read as it is shown.
    pub(super) marking: bool,
}
