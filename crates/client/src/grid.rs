//! The grid's list session: the list it shows, the rows it is laid out in, and what a change of the list or of an item
//! sends to the view. The desktop does the Qt part. A change is made in two steps, as Qt wants it: `plan` makes the
//! new rows without touching the state, the desktop opens the model's insert, remove or reset around `commit`, which
//! puts them in and returns the rows that changed in place, and the desktop emits a data change for each of those.
//! The list session (which list is current, which waits for the viewer, which files were edited while it loaded)
//! is here too; where a file has to be read again, the caller gives the read as a closure.

use std::ops::Range;

use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::turns::Turns;

use crate::layout::{self, Placement, Row, RowChange, RowUpdate};

/// What changed in the items since the rows were last shown.
#[derive(Clone, Copy, Debug)]
pub enum Changed<'a> {
    /// Nothing: a new size.
    Nothing,
    /// These items (turned, rewritten).
    Items(&'a [usize]),
    /// The list was read again (the same folder): what it was.
    List(&'a [Entry]),
    /// Another list: every row is made again.
    All,
}

/// How the model's rows change, for the desktop to open the matching model change around `commit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Every row is made again: the view goes back to the top.
    Reset,
    /// These rows are inserted, in the new rows' numbering.
    Insert(Range<usize>),
    /// These rows of the old rows are removed.
    Remove(Range<usize>),
    /// The same rows, some of them changed in place (`Update`).
    Same,
}

/// A row that changed in place, in the new rows' numbering, and which of its roles the view is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub rows: Range<usize>,
    /// Its span (its first item and count) changed.
    pub span: bool,
    /// Its items changed, so it has a new version and its cells read their items again.
    pub version: bool,
    /// The height role is sent with the others: its height changed, or its span changed, or its items changed and
    /// its height didn't (a row whose only change is its items is sent its version alone).
    pub height: bool,
}

/// A layout made but not yet put in. `Grid::commit` puts it in, and it must be committed before the grid changes
/// again: it was made against the rows as they are now.
pub struct Plan {
    rows: Vec<Row>,
    change: RowChange,
    updates: Vec<(Range<usize>, RowUpdate)>,
    /// The version the changed rows get.
    version: u32,
    reset: bool,
}

impl Plan {
    /// How the model's rows change.
    pub fn shape(&self) -> Shape {
        let at = self.change.changed.end;
        if self.reset {
            Shape::Reset
        } else if self.change.inserted > 0 {
            Shape::Insert(at..at + self.change.inserted)
        } else if self.change.removed > 0 {
            Shape::Remove(at..at + self.change.removed)
        } else {
            Shape::Same
        }
    }
}

/// What a refresh of the items still waiting for a turn found (`Grid::refresh_turned`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Turned {
    /// The items whose entries changed.
    pub changed: Vec<usize>,
    /// Whether one of them changed shape, which changes the rows.
    pub reshaped: bool,
}

/// Whether a list asked for has come in (`Grid::arrive`).
#[derive(Debug)]
pub enum Arrival {
    /// A newer list was asked for since: this one is dropped.
    Stale,
    /// The viewer is open: it is shown when the viewer closes (`Grid::release_held`).
    Held,
    /// Show these entries, with the files edited while it loaded read again.
    Ready(Vec<Entry>),
}

/// Where the selection goes once a list is shown (`Grid::selection_after`): the item current and the anchor, if any.
#[derive(Debug, PartialEq, Eq)]
pub struct Selection {
    pub current: Option<usize>,
    pub anchor: Option<usize>,
}

#[derive(Default)]
pub struct Grid {
    entries: Vec<Entry>,
    rows: Vec<Row>,
    /// The last layout's rows, kept for their capacity: a layout is made into it, then swapped in.
    spare_rows: Vec<Row>,
    /// Per row: a version never given before, whenever the row's items changed without its span changing.
    versions: Vec<u32>,
    /// The last version given.
    last_version: u32,
    spacing: f64,
    width: f64,
    row_height: f64,
    /// The latest list asked for; older ones arriving are dropped.
    generation: u64,
    /// Whether it keeps the selection: every request since the last list shown was a refresh of the same folder.
    keep: bool,
    /// A list that arrived while the viewer was open (its generation, its entries).
    held: Option<(u64, Vec<Entry>)>,
    /// Files rated, turned or marked while the list was loading: its snapshot may predate them.
    edited: Vec<FileId>,
    /// Files were turned while the list was loading: the list may predate the turns, so turns are read again.
    turned: bool,
    /// How many entries are bordered (`is_bordered`), kept as the entries change (`swap_entries`, `edit`,
    /// `refresh_turned`), so the bar's count is not a scan per change.
    bordered: usize,
}

/// Whether an entry counts as bordered: its borders are found and not marked as not a border (what cropping cuts).
fn is_bordered(entry: &Entry) -> bool {
    entry.borders == Borders::Found
}

impl Grid {
    /// The items, in the order the list has them.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The item at an index (`None` outside the list).
    pub fn item(&self, item: usize) -> Option<&Entry> {
        self.entries.get(item)
    }

    /// The number of items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The rows the items are laid out in.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// A row's version (`Grid::plan`, `Grid::touch`); 0, a version never given, for a row past the rows.
    pub fn version(&self, row: usize) -> u32 {
        self.versions.get(row).copied().unwrap_or(0)
    }

    /// The file of an item (`None` outside the list).
    pub fn id_at(&self, item: usize) -> Option<FileId> {
        self.item(item).map(|e| e.id)
    }

    /// The index of a file in the list; `None` when it is not there.
    pub fn position_of(&self, id: FileId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// The layout's rows as they are placed: the items' clamped aspects, read from the entries (1 past them).
    pub fn placement(&self) -> Placement<'_, impl Fn(usize) -> f64 + '_> {
        Placement {
            rows: &self.rows,
            spacing: self.spacing,
            len: self.entries.len(),
            aspect: |i: usize| self.entries.get(i).map_or(1.0, |e| layout::clamp_aspect(e.aspect())),
        }
    }

    /// The row of an item.
    pub fn row_index(&self, item: usize) -> usize {
        self.placement().row_index(item)
    }

    /// The size the rows are laid out at, for the next `plan`.
    pub fn set_size(&mut self, width: f64, row_height: f64, spacing: f64) {
        (self.width, self.row_height, self.spacing) = (width, row_height, spacing);
    }

    fn new_version(&mut self) -> u32 {
        self.last_version += 1;
        self.last_version
    }

    /// The new rows for a change. Nothing is changed: the rows are made against the current ones, and `commit` puts
    /// them in.
    pub fn plan(&mut self, changed: Changed) -> Plan {
        let mut rows = std::mem::take(&mut self.spare_rows);
        if self.width > 0.0 && self.row_height > 0.0 {
            let (width, height, spacing) = (self.width, self.row_height, self.spacing);
            layout::justify_into(&mut rows, self.entries.iter().map(Entry::aspect), width, height, spacing);
        } else {
            rows.clear();
        }
        let change = RowChange::between(&self.rows, &rows);
        let reset = matches!(changed, Changed::All);
        let updates = if reset {
            Vec::new()
        } else {
            change.updates(&self.rows, &rows, |span| match changed {
                Changed::Nothing | Changed::All => false,
                Changed::Items(items) => items.iter().any(|i| span.contains(i)),
                Changed::List(shown) => shown.get(span.clone()) != self.entries.get(span),
            })
        };
        let version = self.new_version();
        Plan { rows, change, updates, version, reset }
    }

    /// Puts a plan in: the new rows, and the versions of the rows inserted, removed or changed. Returns the rows that
    /// changed in place, for the view to be told about; a reset or an insert or remove has none beyond its own.
    pub fn commit(&mut self, plan: Plan) -> Vec<Update> {
        let Plan { rows, change, updates, version, reset } = plan;
        self.spare_rows = std::mem::replace(&mut self.rows, rows);
        if reset {
            self.versions.clear();
            self.versions.resize(self.rows.len(), version);
            return Vec::new();
        }
        let at = change.changed.end;
        if change.inserted > 0 {
            self.versions.splice(at..at, std::iter::repeat_n(version, change.inserted));
        } else if change.removed > 0 {
            self.versions.drain(at..at + change.removed);
        }
        updates
            .into_iter()
            .map(|(rows, u)| {
                if u.items
                    && let Some(versions) = self.versions.get_mut(rows.clone())
                {
                    versions.fill(version);
                }
                let height = u.span || !u.items || u.height;
                Update { rows, span: u.span, version: u.items, height }
            })
            .collect()
    }

    /// Items changed (rated, turned, marked) without changing shape: their rows get a new version. Returns those
    /// rows, for the view to read their items again.
    pub fn touch(&mut self, items: &[usize]) -> Vec<usize> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let last = self.rows.len() - 1;
        let mut rows: Vec<usize> = items.iter().map(|&i| self.row_index(i).min(last)).collect();
        rows.sort_unstable();
        rows.dedup();
        let version = self.new_version();
        for &row in &rows {
            if let Some(v) = self.versions.get_mut(row) {
                *v = version;
            }
        }
        rows
    }

    /// Every row gets a new version, for a change that every row's items must be read again for (the thumbnails'
    /// height): returns the number of rows.
    pub fn touch_all(&mut self) -> usize {
        let version = self.new_version();
        self.versions.fill(version);
        self.rows.len()
    }

    /// Asks for a list: `generation` is the controller's for it (`Command::OpenList`, newer than any asked before),
    /// which the list must still have to be shown. A refresh of the folder shown keeps the selection, unless a folder
    /// change is still on its way (`loading`): a refresh asked meanwhile mustn't keep a selection made in another
    /// folder.
    pub fn request(&mut self, keep: bool, loading: bool, generation: u64) {
        self.generation = generation;
        self.keep = keep && (self.keep || !loading);
        self.edited.clear();
        self.turned = false;
    }

    /// The generation of the list asked for last (`request`).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the selection is kept over the list asked for last (`request`).
    pub fn keep(&self) -> bool {
        self.keep
    }

    /// A list came in. It is dropped if a newer one was asked for since; held while the viewer is open; otherwise
    /// the files edited while it loaded are read again (`reload`: the file's entry now, `None` when it is gone or
    /// unreadable) and it is ready to show.
    pub fn arrive(
        &mut self,
        generation: u64,
        mut entries: Vec<Entry>,
        viewing: bool,
        mut reload: impl FnMut(FileId) -> Option<Entry>,
    ) -> Arrival {
        if generation != self.generation {
            return Arrival::Stale;
        }
        if viewing {
            self.held = Some((generation, entries));
            return Arrival::Held;
        }
        for id in std::mem::take(&mut self.edited) {
            if let Some(slot) = entries.iter_mut().find(|e| e.id == id)
                && let Some(fresh) = reload(id)
            {
                *slot = fresh;
            }
        }
        Arrival::Ready(entries)
    }

    /// The list that arrived while the viewer was open, when the viewer closes: to `arrive` again.
    pub fn release_held(&mut self) -> Option<(u64, Vec<Entry>)> {
        self.held.take()
    }

    /// Whether the list asked for last (`request`) has arrived and waits under the viewer: loaded, not yet shown. A
    /// held list asked for before a newer request doesn't count: that one is still loading.
    pub fn is_held(&self) -> bool {
        matches!(self.held, Some((generation, _)) if generation == self.generation)
    }

    /// Puts a list in as the items, and returns the ones it replaces (the layout compares them, `Changed::List`).
    pub fn swap_entries(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        self.bordered = entries.iter().filter(|e| is_bordered(e)).count();
        std::mem::replace(&mut self.entries, entries)
    }

    /// How many of the items are bordered (found and not marked): what cropping them would cut.
    pub fn bordered(&self) -> usize {
        self.bordered
    }

    /// Changes an item the user edited (rated, turned, marked). While the list loads, the file is noted, so it is
    /// read again when the list arrives.
    pub fn edit(&mut self, item: usize, loading: bool, change: impl FnOnce(&mut Entry)) {
        let Some(e) = self.entries.get_mut(item) else { return };
        let id = e.id;
        let before = is_bordered(e);
        change(&mut *e);
        let after = is_bordered(e);
        self.count_bordered(before, after);
        if loading {
            self.edited.push(id);
        }
    }

    /// One entry's bordered state went from `before` to `after`.
    fn count_bordered(&mut self, before: bool, after: bool) {
        match (before, after) {
            (false, true) => self.bordered += 1,
            (true, false) => self.bordered -= 1,
            _ => {}
        }
    }

    /// Files were turned, while the list loads: the list on its way may predate the turns.
    pub fn note_turns_written(&mut self, loading: bool) {
        if loading {
            self.turned = true;
        }
    }

    /// Whether turns were written while the list loaded, and resets that.
    pub fn take_turned(&mut self) -> bool {
        std::mem::take(&mut self.turned)
    }

    /// The items still waiting for a turn (their rotation is queued, not yet written) are read again, by `reload`
    /// (the file's entry now, `None` when it is gone or unreadable). Only the ones whose entries changed are
    /// reported.
    pub fn refresh_turned(&mut self, mut reload: impl FnMut(FileId) -> Option<Entry>) -> Turned {
        let mut turned = Turned::default();
        // Each changed entry's bordered state, before and after, counted once the entries are let go.
        let mut bordered = Vec::new();
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.pending_turns == Turns::NONE {
                continue;
            }
            let Some(fresh) = reload(entry.id) else { continue };
            if fresh == *entry {
                continue;
            }
            turned.reshaped |= fresh.aspect() != entry.aspect();
            bordered.push((is_bordered(entry), is_bordered(&fresh)));
            *entry = fresh;
            turned.changed.push(i);
        }
        for (before, after) in bordered {
            self.count_bordered(before, after);
        }
        turned
    }

    /// Where the selection goes once the list `self` holds is shown: the file that was current, or what took its
    /// place when it left the list; the anchor only when the selection is kept. `old_current` is the index it had.
    pub fn selection_after(
        &self,
        keep: bool,
        old_current: Option<usize>,
        current: Option<FileId>,
        anchor: Option<FileId>,
    ) -> Selection {
        let current = match (keep, old_current) {
            (false, _) | (true, None) => None,
            (true, Some(old)) => current
                .and_then(|id| self.position_of(id))
                .or_else(|| self.entries.len().checked_sub(1).map(|last| old.min(last))),
        };
        let anchor = if keep { anchor.and_then(|id| self.position_of(id)) } else { None };
        Selection { current, anchor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::grid::Borders;
    use pg_model::media::Ext;
    use pg_model::orientation::Orientation;
    use pg_model::rating::Rating;

    /// A landscape image (or portrait, when `h > w`) of file `id`.
    fn entry(id: i64, w: u32, h: u32) -> Entry {
        Entry {
            id: FileId::new(id),
            mtime: 0,
            width: w,
            height: h,
            duration_ms: Entry::NO_DURATION,
            ext: Ext::of("jpg").unwrap(),
            rotation: Turns::NONE,
            pending_turns: Turns::NONE,
            rating: None,
            animated: false,
            orientation: Orientation::UPRIGHT,
            cut: [0; 4],
            borders: Borders::Unknown,
        }
    }

    /// Lays `entries` out as a list, the way the desktop does: a reset, then committed.
    fn show(g: &mut Grid, entries: Vec<Entry>) {
        g.swap_entries(entries);
        let plan = g.plan(Changed::All);
        assert_eq!(plan.shape(), Shape::Reset);
        assert!(g.commit(plan).is_empty());
    }

    /// Square items, three to a row at this size: 3 items are one row, 4 are two.
    fn square_grid(entries: Vec<Entry>) -> Grid {
        let mut g = Grid::default();
        g.set_size(300.0, 100.0, 0.0);
        show(&mut g, entries);
        g
    }

    #[test]
    fn a_reset_gives_every_row_one_version() {
        let g = square_grid((1..=4).map(|i| entry(i, 100, 100)).collect());
        assert_eq!(g.rows().len(), 2);
        assert_eq!(g.version(0), g.version(1));
    }

    #[test]
    fn a_refresh_with_the_same_items_changes_nothing() {
        let mut g = square_grid((1..=4).map(|i| entry(i, 100, 100)).collect());
        let shown = g.entries().to_vec();
        let plan = g.plan(Changed::List(&shown));
        assert_eq!(plan.shape(), Shape::Same);
        assert!(g.commit(plan).is_empty());
    }

    #[test]
    fn a_list_that_grows_inserts_the_rows_it_adds_and_keeps_the_others_versions() {
        let mut g = square_grid((1..=3).map(|i| entry(i, 100, 100)).collect());
        let before = g.version(0);
        let shown = g.swap_entries((1..=4).map(|i| entry(i, 100, 100)).collect());
        let plan = g.plan(Changed::List(&shown));
        assert_eq!(plan.shape(), Shape::Insert(1..2));
        assert!(g.commit(plan).is_empty());
        assert_eq!(g.rows().len(), 2);
        assert_eq!(g.version(0), before);
    }

    #[test]
    fn an_item_turned_into_another_shape_changes_its_row_span_and_height_together() {
        let mut g = square_grid((1..=4).map(|i| entry(i, 100, 100)).collect());
        // Item 1 becomes wide: its row's span changes, so the row's span and height are sent, not a version alone.
        let mut wide = g.entries().to_vec();
        wide[0] = entry(1, 300, 100);
        let shown = g.swap_entries(wide);
        let plan = g.plan(Changed::List(&shown));
        assert_eq!(plan.shape(), Shape::Same);
        let updates = g.commit(plan);
        assert!(updates.iter().any(|u| u.span && u.height && !u.version), "{updates:?}");
    }

    #[test]
    fn an_item_changed_in_place_gives_its_row_a_version_alone() {
        let mut g = square_grid((1..=4).map(|i| entry(i, 100, 100)).collect());
        let before = (g.version(0), g.version(1));
        let shown = g.entries().to_vec();
        let mut rated = shown.clone();
        rated[3].rating = Rating::new(4);
        g.swap_entries(rated);
        let plan = g.plan(Changed::List(&shown));
        let updates = g.commit(plan);
        assert_eq!(
            updates,
            vec![Update { rows: 1..2, span: false, version: true, height: false }],
            "only the row of the changed item"
        );
        assert_eq!(g.version(0), before.0, "the other row keeps its version");
        assert_ne!(g.version(1), before.1);
    }

    #[test]
    fn touch_gives_the_rows_of_the_items_a_new_version() {
        let mut g = square_grid((1..=4).map(|i| entry(i, 100, 100)).collect());
        let (v0, v1) = (g.version(0), g.version(1));
        assert_eq!(g.touch(&[3, 0]), vec![0, 1]);
        assert_ne!(g.version(0), v0);
        assert_ne!(g.version(1), v1);
        assert!(Grid::default().touch(&[0]).is_empty());
    }

    #[test]
    fn a_request_keeps_the_selection_only_for_a_refresh_of_the_list_shown() {
        let mut g = Grid::default();
        g.request(false, false, 1);
        assert!(!g.keep());
        // A refresh asked while a folder change is still loading doesn't keep the selection made in that folder.
        g.request(true, true, 2);
        assert!(!g.keep());
        // Once nothing is loading, a refresh keeps it.
        g.request(true, false, 3);
        assert!(g.keep());
        // The generation is the one given: the controller's, never one of the grid's own.
        assert_eq!(g.generation(), 3);
    }

    #[test]
    fn a_stale_list_is_dropped_and_a_list_under_the_viewer_is_held_until_it_closes() {
        let mut g = Grid::default();
        let (old, new) = (1, 2);
        g.request(false, false, old);
        g.request(false, false, new);
        assert!(matches!(g.arrive(old, vec![entry(1, 1, 1)], false, |_| None), Arrival::Stale));
        let list = vec![entry(2, 1, 1)];
        assert!(matches!(g.arrive(new, list.clone(), true, |_| None), Arrival::Held));
        assert_eq!(g.release_held(), Some((new, list.clone())));
        assert_eq!(g.release_held(), None);
        assert!(matches!(g.arrive(new, list.clone(), false, |_| None), Arrival::Ready(l) if l == list));
    }

    #[test]
    fn a_held_list_counts_as_held_only_while_it_is_the_newest_request() {
        let mut g = Grid::default();
        let first = 1;
        g.request(false, false, first);
        assert!(!g.is_held());
        assert!(matches!(g.arrive(first, vec![entry(1, 1, 1)], true, |_| None), Arrival::Held));
        assert!(g.is_held());
        // A newer list is asked for: the held one is stale and the newer one is still loading.
        let second = 2;
        g.request(false, true, second);
        assert!(!g.is_held());
        assert!(matches!(g.arrive(second, vec![entry(2, 1, 1)], false, |_| None), Arrival::Ready(_)));
        assert!(!g.is_held());
    }

    #[test]
    fn files_edited_while_the_list_loaded_are_read_again_when_it_arrives() {
        let mut g = Grid::default();
        let generation = 7;
        g.request(false, false, generation);
        g.swap_entries(vec![entry(1, 1, 1), entry(2, 1, 1)]);
        // Item 2 is rated while the list is still loading (loading is the desktop's flag).
        g.edit(1, true, |e| e.rating = Rating::new(5));
        let fresh = Entry { rating: Rating::new(3), ..entry(2, 1, 1) };
        let Arrival::Ready(list) = g.arrive(generation, vec![entry(1, 1, 1), entry(2, 1, 1)], false, |id| {
            assert_eq!(id, FileId::new(2));
            Some(fresh)
        }) else {
            panic!("the list is ready");
        };
        assert_eq!(list[1], fresh);
        assert_eq!(list[0], entry(1, 1, 1));
    }

    #[test]
    fn a_turn_written_while_loading_is_noted_for_the_next_list() {
        let mut g = Grid::default();
        g.note_turns_written(false);
        assert!(!g.take_turned());
        g.note_turns_written(true);
        assert!(g.take_turned());
        assert!(!g.take_turned());
    }

    #[test]
    fn a_refresh_of_turns_reports_the_items_whose_entries_changed_and_whether_one_changed_shape() {
        // A 100 × 50 file waiting for a quarter turn: the grid already shows it as 50 × 100 (its aspect is 0.5).
        let mut waiting = entry(1, 100, 50);
        waiting.pending_turns = Turns::new(1);
        let mut g = Grid::default();
        g.swap_entries(vec![entry(9, 1, 1), waiting, entry(3, 1, 1)]);
        // The turn is written: the file is stored turned, so it shows the same shape and no row changes.
        let turned = g.refresh_turned(|id| {
            assert_eq!(id, FileId::new(1));
            Some(Entry { rotation: Turns::new(1), ..entry(1, 50, 100) })
        });
        assert_eq!(turned, Turned { changed: vec![1], reshaped: false });
        // Nothing waits any more, so nothing is read.
        assert_eq!(g.refresh_turned(|_| None), Turned::default());

        // Another turn that stores the file unturned shows it as 100 × 50 instead: its row changes.
        let mut waiting = entry(2, 100, 50);
        waiting.pending_turns = Turns::new(1);
        g.swap_entries(vec![waiting]);
        let turned = g.refresh_turned(|_| Some(entry(2, 100, 50)));
        assert_eq!(turned, Turned { changed: vec![0], reshaped: true });
    }

    #[test]
    fn the_selection_follows_its_file_or_takes_the_place_of_a_removed_one() {
        let mut g = Grid::default();
        g.swap_entries(vec![entry(1, 1, 1), entry(2, 1, 1), entry(3, 1, 1)]);
        // Kept, the file is there: it's selected where it is now; the anchor too.
        assert_eq!(
            g.selection_after(true, Some(1), Some(FileId::new(2)), Some(FileId::new(1))),
            Selection { current: Some(1), anchor: Some(0) }
        );
        // Kept, the file left the list: the item at its old index (clamped), and no anchor.
        g.swap_entries(vec![entry(1, 1, 1), entry(3, 1, 1)]);
        assert_eq!(
            g.selection_after(true, Some(1), Some(FileId::new(2)), Some(FileId::new(9))),
            Selection { current: Some(1), anchor: None }
        );
        // Not kept: nothing is selected.
        assert_eq!(
            g.selection_after(false, Some(1), Some(FileId::new(3)), None),
            Selection { current: None, anchor: None }
        );
    }
}
