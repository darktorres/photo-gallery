//! The start of the window: the first list is shown with the file the last session left selected, the folder tree is in,
//! and then the start is over (`Started`: the test hooks run and the background job starts). The gate needs only facts
//! the desktop reports, in any order: the tree first or the list first, a list that waits for the grid's width, and a
//! folder the tree no longer has (then all folders are listed, and the start waits for that list).
//!
//! The listed folder is the view's, which is the saved one unless a saved view was applied: nothing here restores a
//! folder, so a view's folder is never replaced by the session's (the `--apply-view` fix).
use serde::{Deserialize, Serialize};

/// What the desktop reports to the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The folder tree was loaded (at the start, and after each scan). `folder_known`: the listed folder is in the
    /// tree (collapsed or not); `""` (all folders) always is.
    TreeLoaded { folder_known: bool },
    /// The list was shown with rows (the grid has a width). `last_file`: the index in the list of the file the last
    /// session left selected, if the list has it.
    ListShown { last_file: Option<usize> },
    /// The list shown is placed in the view: the start's last part of the list.
    ListPositioned,
}

/// What the gate asks the desktop to do, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// The listed folder is gone: list all folders. The start waits for that list.
    OpenAll,
    /// Select this item of the list (the session's last file).
    Select(usize),
    /// The start is over: the tree and the first list are in.
    Started,
}

/// Where the start is. Once `Started` is sent, the gate sends nothing more of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartGate {
    /// The tree has been loaded.
    tree: bool,
    /// A list is shown and placed, since the last `OpenAll` (a list asked again for all folders must come first).
    list: bool,
    /// A list was shown, and its placement has not been reported yet.
    shown: bool,
    started: bool,
}

impl StartGate {
    /// The gate's commands for one event, in the order to carry them out.
    pub fn update(&mut self, event: Event) -> Vec<Command> {
        let mut out = Vec::new();
        match event {
            Event::TreeLoaded { folder_known: false } => {
                self.tree = true;
                self.list = false;
                self.shown = false;
                out.push(Command::OpenAll);
                return out;
            }
            Event::TreeLoaded { folder_known: true } => self.tree = true,
            Event::ListShown { last_file } => {
                self.shown = true;
                if !self.started {
                    out.extend(last_file.map(Command::Select));
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
