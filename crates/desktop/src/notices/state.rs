//! The state of the `Notices` QObject: the bridge's backing struct (the entries shown, the badge, the passes linked and
//! the toast).

use cxx_qt_lib::QString;
use pg_model::inbox::InboxEntry;

#[expect(
    unreachable_pub,
    reason = "the bridge's generated code names it in a public interface (E0446), so it is `pub`"
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
    /// The selected entry's key (its `key` role), empty when no entry is selected. The selection is held by key, so it
    /// follows its entry as the rows move.
    pub(crate) selected_key: QString,
    /// The selected entry's details are open (the panel shows them).
    pub(crate) details_open: bool,
    /// The selection as the panel reads it: the selected entry's row (-1 when none), its body, its pass (-1 for a kept
    /// notice), its newest failure, and the two actions it offers.
    pub(crate) selected_row: i32,
    pub(crate) selected_text: QString,
    pub(crate) selected_pass: i32,
    pub(crate) selected_serial: f64,
    pub(crate) selected_can_retry: bool,
    pub(crate) selected_can_show_files: bool,
    /// The (key, serial) of the failed-file list last asked for, so it is asked once per entry and failure.
    pub(super) requested: (QString, f64),
}

impl Default for NoticesRust {
    fn default() -> Self {
        Self {
            unread: 0,
            passes: 0,
            toast: QString::default(),
            toast_level: 0,
            rows: Vec::new(),
            started: false,
            viewing: false,
            marking: false,
            selected_key: QString::default(),
            details_open: false,
            selected_row: -1,
            selected_text: QString::default(),
            selected_pass: -1,
            selected_serial: 0.0,
            selected_can_retry: false,
            selected_can_show_files: false,
            requested: (QString::default(), 0.0),
        }
    }
}
