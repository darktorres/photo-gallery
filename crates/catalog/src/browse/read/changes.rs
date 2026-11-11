//! The files changed since a change number (`files.changed`): what a client that kept its list as of that number has to
//! fetch again. Every user-visible change stamps its file, in the same statement as the change (the schema's triggers):
//! the file itself, its analysis (made, changed or removed), the borders found in it (the cut and the sides), its
//! rating, its border mark, and its queued write.
//!
//! No caller in the app yet: its planned consumer is a `Bus` subscriber that fell behind (`Lagged`) and resyncs
//! through it (docs/REFACTOR_PLAN.md, the event primitives), which is not built.

use crate::error::Result;
use pg_model::ids::FileId;

use crate::Catalog;

impl Catalog {
    /// The files changed after change number `since`, in id order, and the catalog's change number now: pass that as
    /// `since` next time.
    pub fn changes_since(&self, since: i64) -> Result<(Vec<FileId>, i64)> {
        // One read snapshot: a change committed between the two reads would otherwise be numbered past the returned
        // change number, and so never reported.
        let snapshot = self.conn.unchecked_transaction()?;
        let now: i64 = snapshot.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |r| r.get(0))?;
        let files = snapshot
            .prepare_cached("SELECT id FROM files WHERE changed > ?1 ORDER BY id")?
            .query_map([since], |r| Ok(FileId::new(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((files, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::library;
    use pg_model::rating::Rating;
    use pg_model::rules::PathRules;
    use pg_model::turns::Turns;

    #[test]
    fn each_user_visible_change_names_its_file() {
        let c = library(PathRules::Windows);
        let (all, now) = c.changes_since(0).unwrap();
        assert!(all.len() >= 4, "every file the library made is a change: {all:?}");
        // Nothing has changed since `now`.
        assert_eq!(c.changes_since(now).unwrap(), (vec![], now));

        c.set_rating(FileId::new(1), Rating::new(3), false).unwrap();
        c.set_border_mark(FileId::new(2), true).unwrap();
        c.queue_rotation(FileId::new(3), Turns::new(1)).unwrap();
        let (changed, after) = c.changes_since(now).unwrap();
        assert_eq!(changed, [FileId::new(1), FileId::new(2), FileId::new(3)]);
        assert!(after > now);
        // What was seen at `after` is not reported again.
        assert_eq!(c.changes_since(after).unwrap(), (vec![], after));

        // A view is not a file's change.
        c.save_view(
            &pg_model::view::SavedView {
                name: "v".into(),
                listing: pg_model::view::Listing {
                    folder: pg_model::rules::Merged::from(vec![pg_model::name::Name::from("p")]),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert_eq!(c.changes_since(after).unwrap(), (vec![], after));
    }
}
