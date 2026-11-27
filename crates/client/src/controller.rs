//! The front end's policy, in one entry: `Controller::update(now_ms, event)` takes what the desktop reports, with the time
//! it happened, and returns the commands to carry out, in order. The controller holds the sub-machines (the start gate,
//! the listing, the grid's input, the panels, the size and the pauses) and routes each event to one of them; nothing here
//! calls Qt or reads a clock. Time is an input: a `now_ms` with each event, so a recorded session replays exactly
//! (tests/replay.rs). The desktop keeps the one controller on the GUI thread and carries the commands out (desktop
//! `controller.rs`, `shell.rs`).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use pg_model::ids::FileId;
use pg_model::rating::Rating;
use pg_model::rules::Merged;
use pg_model::size::{GridWidth, RowHeight};
use pg_model::turns::Turns;
use pg_model::view::{Listing, SavedView};
use serde::{Deserialize, Serialize};

use crate::keys::{Global, GridKey};

pub mod deadlines;
pub mod input;
pub mod listing;
pub mod panels;
pub mod replay;
pub mod size;
pub mod start;

use deadlines::{Deadlines, Timer};
use input::Input;
use panels::{PanelKind, Panels};
use size::GridSize;
use start::StartGate;

/// The space between the grid's rows and between its cells, in pixels.
pub const GAP: f64 = 4.0;

/// The state a window starts with: the listing it shows, the row heights (the one it shows, and the one the prefs saved),
/// and the panels' flags. Nothing here is a save: a start's values (a `--size`, `--borders=1`, `--dev=1`) hold for the
/// run, and the controller saves only what the user changes afterwards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Start {
    /// The grid's listing: the folder, the query and the sort.
    pub listing: Listing,
    /// The grid's row height for this run.
    pub row: RowHeight,
    /// The row height saved for the next start.
    pub saved_row: RowHeight,
    /// Whether the filter bar is open.
    pub filters_open: bool,
    /// Whether the borders are shown.
    pub show_borders: bool,
    /// Whether developer mode is on.
    pub dev_mode: bool,
    /// The session's last file, selected in the first list (the prefs'). `None` when none was remembered.
    pub last_file: Option<FileId>,
    /// The listing came from a saved view a start applied (`--apply-view`): the controller remembers it as the session
    /// once the start is over, since that is what applying a view leaves.
    pub apply_view: bool,
}

impl Start {
    /// A start with the listing and the rows, and every panel off, no last file, and no view applied.
    pub fn new(listing: Listing, row: RowHeight, saved_row: RowHeight) -> Start {
        Start {
            listing,
            row,
            saved_row,
            filters_open: false,
            show_borders: false,
            dev_mode: false,
            last_file: None,
            apply_view: false,
        }
    }
}

/// What the desktop reports. The listing's events carry a whole saved view, which is larger than the rest, so they are
/// boxed: each event is the size of its largest variant, and the listing's are rare (a user's choice). A box serialises
/// as its contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// The folder tree was loaded (at the start, and after each scan).
    TreeLoaded {
        /// Whether the listed folder is in the tree.
        folder_known: bool,
    },
    /// The list was shown with `count` rows. `generation` is the [`Command::OpenList`] whose list it is (the one the
    /// model applied last); `None` when no list was applied yet, and in a recording made before it was reported.
    ListShown {
        /// How many rows the list has.
        count: usize,
        /// The list's generation, as the command named it.
        #[serde(default)]
        generation: Option<u64>,
    },
    /// The list shown is placed in the view: the start's last part.
    ListPositioned,
    /// The selection (the current item) changed, or none is selected now.
    CurrentChanged(Option<usize>),
    /// The window selects an item (a test hook, or the viewer closing on it).
    Select(usize),
    /// Search for similar pictures of a catalog file (the viewer's button).
    FindSimilar(FileId),
    /// Search for similar pictures of the command line's photo (`--find-similar`).
    FindSimilarCommandLine,
    /// A grid key, already decided by the key map against the current item.
    Key(GridKey),
    /// The grid's width changed, in pixels. `None` when the list has no width yet (the shell's width of zero or less).
    GridResized(Option<GridWidth>),
    /// The filter bar's query was typed: the list is shown once the typing rests.
    QueryEdited,
    /// A panel opened, closed or toggled: the filter bar, the borders shown, the viewer, dev mode as the window starts
    /// with it, a modal panel (the desktop's menus, bars, keys and panels).
    Panel(panels::Event),
    /// A window shortcut (`GlobalShortcuts.qml`): the log view, dev mode.
    Global(Global),
    /// A size request: the grid's row height stepped, scaled or chosen, or the screen's scale changed.
    Size(size::Event),
    /// Where the grid looks changed by the user: the folder, the recursion, the sort, the query, a saved view.
    Listing(Box<listing::Event>),
    /// A rating, a quarter turn, or a border mark, asked for by the user (the viewer's or the grid's keys).
    Rate {
        /// The item rated.
        item: usize,
        /// `None` takes the rating away (unrated).
        rating: Option<Rating>,
    },
    /// A rating made in the viewer: the item is rated, and the viewer moves on to the next one.
    RateInViewer {
        /// The item rated.
        item: usize,
        /// The rating set, or `None` to take it away.
        rating: Option<Rating>,
    },
    /// A quarter turn of an item, asked for by the user.
    Turn {
        /// The item turned.
        item: usize,
        /// Quarter turns clockwise (a counterclockwise turn is three).
        turns: Turns,
    },
    /// Toggles an item's border mark, asked for by the user.
    ToggleBorderMark(usize),
    /// An item's border mark is set (the viewer's own state, as the window reports it).
    SetBorderMark {
        /// The item whose mark is set.
        item: usize,
        /// Whether it is marked.
        marked: bool,
    },
    /// The folder the window spelled (its canonical name): the listing's folder is replaced, nothing is listed, and the
    /// session is remembered.
    FolderSpelled(Merged),
    /// Show the file `file` (an inbox entry's Reveal): its folder, `folder`, is listed with nothing filtered, and the file
    /// is selected once the list is shown ([`Command::Reveal`]).
    Reveal {
        /// The file to show.
        file: FileId,
        /// Its folder, as the window spells it.
        folder: Merged,
    },
    /// The list is read again, the selection kept (the files changed on disk).
    Reload,
    /// Files were turned in place on disk: their rows are read again (no renumbering).
    FilesChanged,
    /// Time has passed: the host's deadline timer fired (or was asked to check). Runs the pauses that are due.
    Timeout,
    /// The window is closing: the pauses that are waiting run now, so no save is lost.
    Flush,
}

/// What the desktop carries out, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Select this item of the list: a key's, or the grid's own (Select for the session's last file at the start).
    Select(usize),
    /// The session's last file is selected, at the start (the list is shown, then this).
    RestoreSelection(FileId),
    /// The start is over: the tree and the first list are in.
    Started,
    /// Move the selection to the neighbour `dx` columns and `dy` rows away (the grid's layout resolves it).
    Step {
        /// Columns away.
        dx: i32,
        /// Rows away.
        dy: i32,
    },
    /// Show the viewer at this item.
    ShowViewer(usize),
    /// List the files of this listing (model: the grid model executes it). keep: the selection is kept, else the list
    /// shows from the top. `generation` names the list: each `OpenList` has a newer one than any before (one counter, the
    /// controller's), the grid shows only the newest it asked for, and the window reports it back when the list is
    /// shown (`Event::ListShown`).
    OpenList {
        /// The listing to show.
        listing: Listing,
        /// Whether the selection is kept.
        keep: bool,
        /// The list's generation, the newest one asked for.
        generation: u64,
    },
    /// Set the rating of an item (model).
    Rate {
        /// The item to rate.
        item: usize,
        /// The rating to set, or `None` to take it away.
        rating: Option<Rating>,
    },
    /// Turn an item by quarter turns (model).
    Turn {
        /// The item to turn.
        item: usize,
        /// The quarter turns.
        turns: Turns,
    },
    /// Toggle an item's border mark (model).
    ToggleBorderMark(usize),
    /// Set an item's border mark (model).
    SetBorderMark {
        /// The item whose mark is set.
        item: usize,
        /// Whether it is marked.
        marked: bool,
    },
    /// Read again the rows of the files turned in place (model).
    RefreshTurned,
    /// Remember the listing for the next start.
    SaveSession(Listing),
    /// Remember the file the session is on (the desktop reads it from the model).
    SaveLastFile,
    /// Lay the grid out again at this width and row height (presentation: the view keeps its place). Only once a width is
    /// known: a layout with no width is not asked for.
    Relayout {
        /// The width the grid is laid out at.
        width: GridWidth,
        /// The row height the grid is laid out at.
        row_height: RowHeight,
    },
    /// A panel changed.
    Panel(panels::Change),
    /// Open this modal panel (a global shortcut).
    OpenPanel(PanelKind),
    /// Close this modal panel (a global shortcut, or a listing that shows a reveal).
    ClosePanel(PanelKind),
    /// Close the viewer: a listing that shows a reveal or the failed files takes it away, as the window's picture of the
    /// list is the grid's.
    CloseViewer,
    /// Remember dev mode for the next start: the user turned it on or off (a start's `--dev=1` is not saved).
    SaveDevMode(bool),
    /// Remember the borders for the next start: the user toggled them (a start's `--borders` is not saved).
    SaveShowBorders(bool),
    /// The size changed: the thumbnails, or the saved row height.
    Size(size::Change),
    /// Select the file `file` in the list just shown, the one a reveal asked for (the model knows its item), or say it is
    /// no longer in the library.
    Reveal(FileId),
    /// The window searches for similar pictures of a catalog file (the window opens its panel).
    FindSimilar(FileId),
    /// The window searches for the command line's photo.
    FindSimilarCommandLine,
}

/// The one controller: every sub-machine, and the pauses.
#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    start: StartGate,
    listing: Listing,
    input: Input,
    panels: Panels,
    grid_size: GridSize,
    deadlines: Deadlines,
    grid_width: Option<GridWidth>,
    /// The last `OpenList`'s generation: the next one is one more.
    generation: u64,
    /// The file a reveal waits to select, and the generation of the list it waits for: only that list's `ListShown`
    /// selects it, and a newer list's drops it (the user went elsewhere).
    reveal: Option<(FileId, u64)>,
    /// Whether the listing is the session's to save: the user changed it, or a view applied at the start is the session.
    /// Until then a selection's save writes the last file alone, so the start's own options never become the session's.
    session_listing: bool,
    /// A selection's save came due before the start was over (and was held back): it is made when the start is over.
    selection_pending: bool,
    /// The start the controller was made from: the recording's `Init`, which replays it.
    initial: Start,
}

impl Controller {
    /// A controller for a window that starts in `start`. The screen's scale arrives as an event.
    pub fn new(start: Start) -> Controller {
        Controller {
            start: StartGate::new(start.last_file),
            listing: start.listing.clone(),
            input: Input::default(),
            panels: Panels::starting(start.filters_open, start.show_borders, start.dev_mode),
            grid_size: GridSize::new(start.row, start.saved_row),
            deadlines: Deadlines::default(),
            grid_width: None,
            generation: 0,
            reveal: None,
            session_listing: start.apply_view,
            selection_pending: false,
            initial: start,
        }
    }

    /// The start the controller was made from.
    pub fn initial(&self) -> &Start {
        &self.initial
    }

    /// The listing as it is now.
    pub fn listing(&self) -> &Listing {
        &self.listing
    }

    /// The panels' flags, as they are now.
    pub fn panels(&self) -> &Panels {
        &self.panels
    }

    /// The grid's current item, if one is selected.
    pub fn current(&self) -> Option<usize> {
        self.input.current()
    }

    /// Whether a grid key that acts on the current item applies.
    pub fn has_current(&self) -> bool {
        self.input.has_current()
    }

    /// The size, as it is now.
    pub fn grid_size(&self) -> &GridSize {
        &self.grid_size
    }

    /// Whether the start is over.
    pub fn started(&self) -> bool {
        self.start.is_started()
    }

    /// The item a viewer step from `from` by `d` lands on.
    pub fn viewer_step(&self, from: usize, d: i32) -> usize {
        self.input.viewer_step(from, d)
    }

    /// Whether any pause is running (the screenshot waits for none).
    pub fn pending(&self) -> bool {
        self.deadlines.pending()
    }

    /// The earliest pause's deadline (ms on the host's clock): the host sets its timer to it.
    pub fn next_deadline(&self) -> Option<u64> {
        self.deadlines.next_deadline()
    }

    /// Apply `event` at `now_ms`, and return the commands, in order.
    pub fn update(&mut self, now_ms: u64, event: Event) -> Vec<Command> {
        match event {
            Event::TreeLoaded { folder_known } => self.start(start::Event::TreeLoaded { folder_known }),
            Event::ListShown { count, generation } => {
                let mut out = self.input_update(input::Event::ListShown { count });
                out.extend(self.start(start::Event::ListShown));
                out.extend(self.reveal_shown(generation));
                out
            }
            Event::ListPositioned => self.start(start::Event::ListPositioned),
            Event::CurrentChanged(current) => {
                self.deadlines.arm(Timer::SaveSession, now_ms);
                self.input_update(input::Event::CurrentChanged(current))
            }
            Event::Key(grid) => self.key(now_ms, grid),
            Event::GridResized(width) => {
                self.grid_width = width;
                self.deadlines.arm(Timer::Layout, now_ms);
                Vec::new()
            }
            Event::QueryEdited => {
                self.deadlines.arm(Timer::ApplyQuery, now_ms);
                Vec::new()
            }
            Event::Panel(event) => self.panel(event),
            Event::Global(global) => self.global(global),
            Event::Size(event) => self.size(now_ms, event),
            Event::Listing(event) => {
                let shows_failures = matches!(*event, listing::Event::FailedImports);
                let out = self.listing_event(*event);
                if shows_failures { self.closed_for_listing(out) } else { out }
            }
            Event::Select(item) => vec![Command::Select(item)],
            Event::FindSimilar(file) => vec![Command::FindSimilar(file)],
            Event::FindSimilarCommandLine => vec![Command::FindSimilarCommandLine],
            Event::Rate { item, rating } => vec![Command::Rate { item, rating }],
            // A rating in the viewer moves on to the next picture, as a rating app's triage does; at the end it stays.
            Event::RateInViewer { item, rating } => {
                let next = self.viewer_step(item, 1);
                let mut out = vec![Command::Rate { item, rating }];
                if next != item {
                    out.push(Command::ShowViewer(next));
                }
                out
            }
            Event::Turn { item, turns } => vec![Command::Turn { item, turns }],
            Event::ToggleBorderMark(item) => vec![Command::ToggleBorderMark(item)],
            Event::SetBorderMark { item, marked } => vec![Command::SetBorderMark { item, marked }],
            Event::FolderSpelled(folder) => {
                self.listing.folder = folder;
                // Remembered only once the start is over, and with the session's listing: until then the listing is the
                // start's, and a spelling the tree gives at the start is not a save (`fire`).
                if self.started() && self.session_listing {
                    vec![Command::SaveSession(self.listing.clone())]
                } else {
                    Vec::new()
                }
            }
            Event::Reveal { file, folder } => {
                let out = self.listing_event(listing::Event::Reveal(folder));
                // The reveal waits for the list it just asked for (always one: a reveal is a listing change).
                self.reveal = out.iter().find_map(|c| match c {
                    Command::OpenList { generation, .. } => Some((file, *generation)),
                    _ => None,
                });
                self.closed_for_listing(out)
            }
            Event::Reload => vec![self.open_list(true)],
            Event::FilesChanged => vec![Command::RefreshTurned],
            Event::Timeout => self.run_due(now_ms),
            Event::Flush => self.flush(),
        }
    }

    /// An `OpenList` of the listing as it is now, with the next generation: every `OpenList` is made here.
    fn open_list(&mut self, keep: bool) -> Command {
        self.generation += 1;
        Command::OpenList { listing: self.listing.clone(), keep, generation: self.generation }
    }

    /// The reveal's `Command::Reveal` when the list shown is the one it waits for; a newer list drops it, an older one
    /// (still shown before the reveal's arrives) leaves it waiting.
    fn reveal_shown(&mut self, generation: Option<u64>) -> Option<Command> {
        let (file, wanted) = self.reveal?;
        // A list shown before any was applied is no list of the reveal's: it waits, as an older list does.
        let generation = generation?;
        if generation < wanted {
            return None;
        }
        self.reveal = None;
        (generation == wanted).then_some(Command::Reveal(file))
    }

    /// The gate's commands for `event`; the one that lists all folders is a listing change, as the window makes it.
    fn start(&mut self, event: start::Event) -> Vec<Command> {
        let mut out = Vec::new();
        for command in self.start.update(event) {
            match command {
                start::Command::OpenAll => out.extend(self.start_listing(listing::Event::OpenAll)),
                start::Command::RestoreSelection(file) => out.push(Command::RestoreSelection(file)),
                start::Command::Started => {
                    // A view applied at the start is what the session is from now on: remembered once, here.
                    if self.initial.apply_view {
                        out.push(Command::SaveSession(self.listing.clone()));
                    }
                    // A selection held back before the start was over is remembered now.
                    if core::mem::take(&mut self.selection_pending) {
                        out.push(Command::SaveLastFile);
                    }
                    out.push(Command::Started);
                }
            }
        }
        out
    }

    /// The listing change the start makes (its fallback to all folders): listed, and not saved, since a start's own
    /// listing is not the user's to remember.
    fn start_listing(&mut self, event: listing::Event) -> Vec<Command> {
        let commands = listing::update(&mut self.listing, event);
        commands
            .into_iter()
            .filter_map(|c| match c {
                listing::Command::OpenList => Some(self.open_list(false)),
                listing::Command::SaveSession => None,
            })
            .collect()
    }

    fn input_update(&mut self, event: input::Event) -> Vec<Command> {
        let commands = self.input.update(event);
        commands.into_iter().filter_map(|c| self.input_command(c)).collect()
    }

    /// The desktop's command for an input command. A model action is on the current item, which the input only
    /// asks for while one is selected; `Forward` never gets here (`key` routes the size and the panels itself).
    fn input_command(&self, command: input::Command) -> Option<Command> {
        Some(match command {
            input::Command::Select(i) => Command::Select(i),
            input::Command::Step { dx, dy } => Command::Step { dx, dy },
            input::Command::ShowViewer(i) => Command::ShowViewer(i),
            input::Command::Act(grid) => {
                let item = self.input.current()?;
                match grid {
                    // The key map rates 0 to 5 only, and 0 takes the rating away (`Rating::new` gives `None`).
                    GridKey::Rate(stars) => Command::Rate { item, rating: Rating::new(stars) },
                    GridKey::Turn { forward } => Command::Turn { item, turns: Turns::of(if forward { 1 } else { -1 }) },
                    _ => Command::ToggleBorderMark(item),
                }
            }
            input::Command::Forward(_) => return None,
        })
    }

    fn key(&mut self, now_ms: u64, grid: GridKey) -> Vec<Command> {
        match grid {
            GridKey::Resize { grow } => self.size(now_ms, size::Event::Resize { grow }),
            GridKey::Filters => self.panel(panels::Event::ToggleFilters),
            GridKey::Borders => self.panel(panels::Event::ToggleBorders),
            _ => self.input_update(input::Event::Key(grid)),
        }
    }

    /// A panel's event: its changes, then the saves the user's own toggle asks for (the borders are remembered for the
    /// next start; the filter bar is not saved).
    fn panel(&mut self, event: panels::Event) -> Vec<Command> {
        let user_borders = event == panels::Event::ToggleBorders;
        let changes = self.panels.update(event);
        let saves: Vec<Command> = changes
            .iter()
            .filter_map(|change| match change {
                panels::Change::ShowBorders(on) if user_borders => Some(Command::SaveShowBorders(*on)),
                _ => None,
            })
            .collect();
        let mut out: Vec<Command> = changes.into_iter().map(Command::Panel).collect();
        out.extend(saves);
        out
    }

    /// A window shortcut. While a modal panel other than the inbox and the log view is open, it is that panel's (a dialog,
    /// a menu): the shortcut does nothing. Otherwise the log view's key closes it when it is open, else opens it,
    /// closing its sibling first (only in dev mode); dev mode's key turns it on or off, and saves it, and closes the
    /// log view when it turns off.
    fn global(&mut self, global: Global) -> Vec<Command> {
        if self.panels.modals().iter().any(|k| !k.is_switchable()) {
            return Vec::new();
        }
        match global {
            Global::Inbox => self.switch_to(PanelKind::Inbox),
            Global::Log if self.panels.dev_mode() => self.switch_to(PanelKind::Log),
            Global::Log => Vec::new(),
            Global::DevMode => {
                let on = !self.panels.dev_mode();
                let mut out = self.panel(panels::Event::SetDevMode(on));
                out.push(Command::SaveDevMode(on));
                if !on && self.panels.is_open(PanelKind::Log) {
                    out.push(Command::ClosePanel(PanelKind::Log));
                }
                out
            }
        }
    }

    /// Close `kind` when it is open; else open it, closing the other switchable panel first.
    fn switch_to(&self, kind: PanelKind) -> Vec<Command> {
        if self.panels.is_open(kind) {
            return vec![Command::ClosePanel(kind)];
        }
        let mut out: Vec<Command> = self
            .panels
            .modals()
            .iter()
            .filter(|k| k.is_switchable() && **k != kind)
            .map(|k| Command::ClosePanel(*k))
            .collect();
        out.push(Command::OpenPanel(kind));
        out
    }

    fn size(&mut self, now_ms: u64, event: size::Event) -> Vec<Command> {
        let changes = self.grid_size.update(event);
        let mut out = Vec::new();
        for change in changes {
            match change {
                size::Change::RowHeight(height) => {
                    self.deadlines.arm(Timer::Size, now_ms);
                    out.push(Command::Size(change));
                    out.extend(self.relayout(height));
                }
                other => out.push(Command::Size(other)),
            }
        }
        out
    }

    /// The relayout at `row_height`, once the grid has a width (none before the list is laid out).
    fn relayout(&self, row_height: RowHeight) -> Option<Command> {
        Some(Command::Relayout { width: self.grid_width?, row_height })
    }

    /// A listing that shows a reveal or the failed files takes the inbox away, and the viewer when it is open: the window
    /// closes them, after the list's own commands.
    fn closed_for_listing(&self, mut out: Vec<Command>) -> Vec<Command> {
        out.push(Command::ClosePanel(PanelKind::Inbox));
        if self.panels.viewer_open() {
            out.push(Command::CloseViewer);
        }
        out
    }

    /// A listing change: applied, then the listing's commands. A query typed is applied at once and shown after its
    /// pause (`QueryEdited`), so the filter bar's changes are not listed one by one.
    fn listing_event(&mut self, event: listing::Event) -> Vec<Command> {
        let query_only = matches!(event, listing::Event::QueryChanged(_));
        let commands = listing::update(&mut self.listing, event);
        // A change made is the session's to save from now on (`fire`).
        self.session_listing |= !commands.is_empty();
        if query_only {
            return Vec::new();
        }
        // The list that is opened now shows the listing as it is: a query's pause still waiting would open the same one.
        if commands.contains(&listing::Command::OpenList) {
            self.deadlines.cancel(Timer::ApplyQuery);
        }
        commands
            .into_iter()
            .map(|c| match c {
                listing::Command::OpenList => self.open_list(false),
                listing::Command::SaveSession => Command::SaveSession(self.listing.clone()),
            })
            .collect()
    }

    /// The pauses that are due at `now_ms`, carried out in the order they were due.
    fn run_due(&mut self, now_ms: u64) -> Vec<Command> {
        let due = self.deadlines.handle_timeout(now_ms);
        self.fire(due)
    }

    /// The shutdown: the saves that wait run now, each once. A query's pause (`ApplyQuery`) saves the session when it
    /// ends, so it waiting is a session to save too: it becomes the session's save (`SaveSession`, which saves the last
    /// file as well), never a list shown (`listing_now`) for a window that is closing. A layout's pause is dropped.
    fn flush(&mut self) -> Vec<Command> {
        let mut save = false;
        let mut due = Vec::new();
        for timer in self.deadlines.drain() {
            match timer {
                Timer::ApplyQuery | Timer::SaveSession => save = true,
                Timer::Size => due.push(timer),
                Timer::Layout => {}
            }
        }
        if save {
            due.push(Timer::SaveSession);
        }
        self.fire(due)
    }

    fn fire(&mut self, due: Vec<Timer>) -> Vec<Command> {
        let mut out = Vec::new();
        for timer in due {
            match timer {
                Timer::ApplyQuery => out.extend(self.listing_now()),
                // Saved only once the start is over: until then the listing is the start's (its options, the remembered
                // file's selection), not the user's, and a save would make a start option the session's. Once it is, the
                // listing is saved only when it is the session's (`session_listing`), and the last file always.
                Timer::SaveSession if self.start.is_started() => {
                    if self.session_listing {
                        out.push(Command::SaveSession(self.listing.clone()));
                    }
                    out.push(Command::SaveLastFile);
                }
                // Held back, and made when the start is over (`start`).
                Timer::SaveSession => self.selection_pending = true,
                Timer::Layout => out.extend(self.relayout(self.grid_size.row_height())),
                Timer::Size => {
                    for change in self.grid_size.settled() {
                        out.push(Command::Size(change));
                    }
                }
            }
        }
        out
    }

    /// The listing's commands for its current state (the query's pause ended).
    fn listing_now(&mut self) -> Vec<Command> {
        vec![self.open_list(false), Command::SaveSession(self.listing.clone())]
    }

    /// A saved view, applied (the listing's change, as the window reports it).
    pub fn apply_view(&mut self, view: SavedView) -> Vec<Command> {
        self.listing_event(listing::Event::ViewApplied(view))
    }
}
