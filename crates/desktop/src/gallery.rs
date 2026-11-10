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
use std::cell::RefCell;

use cxx_qt::{CxxQtThread, Threading};

thread_local! {
    /// The model's thread handle, kept at `register`. Only the GUI thread touches it.
    static MODEL: RefCell<Option<CxxQtThread<qobject::GalleryModel>>> = const { RefCell::new(None) };
}

/// Carry out `run` on the grid model, after the events already queued to it: the controller's model commands (rate,
/// turn, list, border marks) come through here. Nothing runs before `register` (the window is not made yet).
pub(crate) fn post(run: impl FnOnce(Pin<&mut qobject::GalleryModel>) + Send + 'static) {
    MODEL.with(|model| {
        if let Some(thread) = model.borrow().as_ref() {
            let _ = thread.queue(run);
        }
    });
}

impl qobject::GalleryModel {
    pub(crate) fn register_model(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        MODEL.with(|model| *model.borrow_mut() = Some(thread));
    }
}
