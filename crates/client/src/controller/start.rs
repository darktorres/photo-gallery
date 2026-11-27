//! The start of the window: the first list is shown with the file the last session left selected, the folder tree is in,
//! and then the start is over (`Started`: the test hooks run and the background job starts). The gate needs only facts
//! the desktop reports, in any order: the tree first or the list first, and a list that waits for the grid's width.
//!
//! The gate never changes the listing. A folder the tree doesn't have is not a reason to list another one: the listing
//! shown stays as it is, its folder included (an absent folder is the folder ledger's and the grid's to show), and a
//! failed tree read (`TreeFailed`) is a state the gate passes, not an absence. So the start is over with the listing the
//! view has, and nothing here saves the session: the controller holds the session's saves until the start is over.
//!
//! The listed folder is the view's, which is the saved one unless a saved view was applied: nothing here restores a
//! folder, so a view's folder is never replaced by the session's (the `--apply-view` fix).
use alloc::vec::Vec;

use pg_model::ids::FileId;
use serde::{Deserialize, Serialize};

/// What the desktop reports to the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The folder tree was loaded (at the start, and after each scan). Whether the listed folder is in it is not the
    /// gate's to decide: a folder the tree doesn't have is listed as it is.
    TreeLoaded,
    /// The folder tree couldn't be read (the failure is a `TreeFailed` notice): the gate treats the tree as in, so the
    /// start goes on with the listing it has, and no other folder is listed in its place.
    TreeFailed,
    /// The list was shown with rows (the grid has a width). Whether the session's last file is in it is the model's to
    /// say: the gate asks for its selection (`RestoreSelection`) and the model selects it if it is there.
    ListShown,
    /// The list shown is placed in the view: the start's last part of the list.
    ListPositioned,
}

/// What the gate asks the desktop to do, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Select the session's last file, if the list shown has it.
    RestoreSelection(FileId),
    /// The start is over: the tree and the first list are in.
    Started,
}

/// Where the start is. Once `Started` is sent, the gate sends nothing more of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartGate {
    /// The session's last file, selected in the first list shown (from the prefs).
    last_file: Option<FileId>,
    /// The tree has been loaded, or its read failed (`TreeFailed`): either way the start doesn't wait for it.
    tree: bool,
    /// A list is shown and placed.
    list: bool,
    /// A list was shown, and its placement has not been reported yet.
    shown: bool,
    started: bool,
}

impl StartGate {
    /// The gate of a start that restores `last_file`'s selection.
    pub fn new(last_file: Option<FileId>) -> StartGate {
        StartGate { last_file, ..StartGate::default() }
    }

    /// The gate's commands for one event, in the order to carry them out.
    pub fn update(&mut self, event: Event) -> Vec<Command> {
        let mut out = Vec::new();
        match event {
            Event::TreeLoaded | Event::TreeFailed => self.tree = true,
            Event::ListShown => {
                self.shown = true;
                if !self.started {
                    out.extend(self.last_file.map(Command::RestoreSelection));
                }
            }
            Event::ListPositioned => {
                if self.shown {
                    self.shown = false;
                    self.list = true;
                }
            }
        }
        if !self.started && self.tree && self.list {
            self.started = true;
            out.push(Command::Started);
        }
        out
    }

    /// Whether the start is over.
    pub fn is_started(&self) -> bool {
        self.started
    }
}
