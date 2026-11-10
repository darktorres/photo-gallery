//! What the app has to say: the notes bus. A [`Note`] is what the user should hear about, besides the results of what they
//! asked for. The front end subscribes ([`Notes::subscribe`]) and prints or shows them; nothing here prints.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use pg_jobs::events::{Bus, Subscription};

use crate::error::Result;

/// What the user should hear about, besides the results of what they asked for. Printed or shown by the front end.
#[derive(Clone, Debug)]
pub enum Note {
    /// The preferences file didn't parse: it was kept aside (`kept`), and the defaults are in use.
    PrefsUnreadable { path: PathBuf, error: String, kept: Result<PathBuf, String> },
    /// The thumbnail cache reported something (a repair, a write it couldn't make, a WAL it couldn't merge).
    Thumbs(pg_thumbcache::Event),
    /// The preferences changed in memory but the file couldn't be written; the change holds until the app closes.
    PrefsNotSaved { path: PathBuf, error: String },
    /// The catalog didn't match this build's schema, so it was moved aside (its `kept` file) and a new one made from this
    /// build's schema, which took the old rows ([`pg_model::notice::Carried`]) or none, with the reason said.
    CatalogRebuilt(pg_catalog::Rebuilt),
}

/// The notes bus. The subscription taken when the app opened is held for the first subscriber, so the notes of the
/// opening are not lost.
pub struct Notes {
    bus: Arc<Bus<Note>>,
    first: Mutex<Option<Subscription<Note>>>,
}

impl Notes {
    /// A bus that holds up to 64 notes for a slow subscriber.
    pub(crate) fn new() -> Notes {
        let bus: Arc<Bus<Note>> = Arc::new(Bus::new(64));
        let first = Mutex::new(Some(bus.subscribe()));
        Notes { bus, first }
    }

    /// What the app has to say, from the first note on: the first call gets the subscription the app held since it opened;
    /// later calls subscribe from now on.
    pub fn subscribe(&self) -> Subscription<Note> {
        self.first.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_else(|| self.bus.subscribe())
    }

    pub(crate) fn publish(&self, note: Note) {
        self.bus.publish(note);
    }
}
