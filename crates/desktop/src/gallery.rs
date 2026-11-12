//! The grid model: the files of the open folder, laid out in justified rows (`pg_client::layout`). QML sees one
//! list row per layout row (virtualized by ListView) and asks for each item's file id and size by index. The
//! model holds a compact entry per file (`pg_model::grid::Entry`); a file's path and size are read when it is shown
//! (`App::details`), for the selection and the viewer's file.
//!
//! The files are listed off the GUI thread (the app's catalog reader): the controller's `OpenList` command asks (the
//! listing says what), and the list arrives later (`loaded`).
//! So that nothing is renumbered under the user, the model keeps the items QML holds: the selection (`current`)
//! moves with its file when a new list arrives, and while the viewer is open (`viewing`) an arriving list waits
//! for it to close. A rating or a turn made while a list was on its way is read again into it.
//!
//! The bridge and the registry (`post`) are here; the model's methods are in its children: [`model`] (the rows
//! and the values the views follow), [`items`] (what it says of one item), [`edits`] and [`nav`], over the roles
//! ([`roles`]) and the backing struct ([`state`]).

mod edits;
mod items;
mod model;
mod nav;
mod roles;
pub(crate) mod state;

use crate::gallery_bridge::qobject;

use core::pin::Pin;
use std::cell::{Cell, RefCell};

use cxx_qt::{CxxQtThread, Threading};
use pg_model::ids::FileId;

thread_local! {
    /// The model's thread handle, kept at `register`. Only the GUI thread touches it.
    static MODEL: RefCell<Option<CxxQtThread<qobject::GalleryModel>>> = const { RefCell::new(None) };
    /// The generation (`Command::OpenList`'s) of the list the model put in last, `None` before any: written by
    /// `GalleryModel::apply` right after it swaps the entries in, and read by `Shell::list_shown` on the same thread,
    /// since the shell reaches the model only through `post`, queued. The one record of what is applied.
    pub(crate) static APPLIED: Cell<Option<u64>> = const { Cell::new(None) };
    /// The file the window shows (the viewer's, while it is open, else the selection's), as the model last set it:
    /// written where either changes (`publish_shown`), read where the session's last file is saved (`shown_file`).
    static SHOWN: Cell<Option<FileId>> = const { Cell::new(None) };
}

/// The generation of the list the grid shows now ([`APPLIED`]); `None` before any was shown.
pub(crate) fn applied() -> Option<u64> {
    APPLIED.with(Cell::get)
}

/// The file the window shows now, `None` for none ([`SHOWN`]).
pub(crate) fn shown_file() -> Option<FileId> {
    SHOWN.with(Cell::get)
}

/// Record the file the window shows, from the model (`GalleryModel::refresh_derived` and `refresh_viewed`).
pub(super) fn publish_shown(file: Option<FileId>) {
    SHOWN.with(|shown| shown.set(file));
}

/// Carry out `run` on the grid model, after the events already queued to it: the controller's model commands (rate,
/// turn, list, border marks) and the thumbnail height come through here. The model registers itself when Qt makes it
/// (`Initialize`), before the window sends it anything; so a `post` with no model is a bug, and is logged, not dropped.
pub(crate) fn post(run: impl FnOnce(Pin<&mut qobject::GalleryModel>) + Send + 'static) {
    MODEL.with(|model| match model.borrow().as_ref() {
        Some(thread) => {
            let _ = thread.queue(run);
        }
        None => pg_log::error("gallery", "a command reached the grid model before it was made; it was not carried out"),
    });
}

impl cxx_qt::Initialize for qobject::GalleryModel {
    /// Make this model the one the window's commands are carried out on (`post`). Qt calls this when the model is made.
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        MODEL.with(|model| *model.borrow_mut() = Some(thread));
    }
}
