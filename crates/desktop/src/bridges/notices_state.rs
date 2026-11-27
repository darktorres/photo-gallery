//! The state of the `Notices` QObject: the bridge's backing struct (the entries shown, the badge, the passes linked and
//! the toast).

use cxx_qt_lib::QString;
use pg_model::inbox::InboxEntry;

pub struct NoticesRust {
    pub(crate) unread: i32,
    pub(crate) passes: i32,
    pub(crate) toast: QString,
    pub(crate) toast_level: i32,
    /// The entries shown, newest first, as the inbox last listed them (less those dismissed since), with the marks made
    /// here since.
    pub(crate) rows: Vec<InboxEntry>,
    /// `start` ran: the inbox is followed.
    pub(crate) started: bool,
    /// The inbox panel is open: the inbox reads the ledgers' counts (`Inbox::set_viewing`).
    pub(crate) viewing: bool,
    /// The panel is open and the window active: what is shown is read by the user, so it is marked read as it is shown.
    pub(crate) marking: bool,
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
    /// The (key, serial, count) of the failed-file list last asked for, so it is asked once per entry and change of it.
    pub(crate) requested: (QString, f64, f64),
    /// The selected ledger's rows in all (`selectedCount`): with the newest serial, what the failed-file list is of. A
    /// file cleared that is not the newest changes the count and not the serial.
    pub(crate) selected_count: f64,
    /// A ledger's Copy is reading its files (`copying`): the clipboard is written from the read, when it is delivered.
    pub(crate) copying: bool,
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
            requested: (QString::default(), 0.0, 0.0),
            selected_count: 0.0,
            copying: false,
        }
    }
}
