//! The grid's input policy (S-12.16): which item is current, and what a grid key does with it. The key map decides what a
//! key means (`keys::grid`); this says what the desktop must do about it. A step to a neighbour needs the model's layout,
//! so it is a command the desktop resolves; the first and the last item are known from the count, so they are resolved
//! here. Facts about the list come in as events. Nothing here calls Qt.

use crate::keys::Grid;

/// A fact about the grid, reported by the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A grid key, already decided by the key map (`keys::grid`) against the current item.
    Key(Grid),
    /// The current item changed (`None`: no item is current).
    CurrentChanged(Option<usize>),
    /// A list of `count` items is shown.
    ListShown { count: usize },
}

/// What the desktop does for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Select this item and scroll it into view.
    Select(usize),
    /// Select the neighbour of the current item `dx` columns and `dy` rows away: the model's layout resolves it.
    Step { dx: i32, dy: i32 },
    /// A model action on the current item (rate, turn, mark its border): the desktop carries out `Grid` for it.
    Act(Grid),
    /// Show the viewer at this item.
    ShowViewer(usize),
    /// Not the grid's to decide: the size or the panels take it (`Shell` passes it on).
    Forward(Grid),
}

/// The grid's current item and the list's count, as the core knows them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    current: Option<usize>,
    count: usize,
}

impl Input {
    /// The current item, if one is selected.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// Whether a grid key that acts on the current item applies (the key map asks this).
    pub fn has_current(&self) -> bool {
        self.current.is_some()
    }

    /// The item a viewer step from `from` by `d` lands on, clamped to the list.
    pub fn viewer_step(&self, from: usize, d: i32) -> usize {
        let last = self.count.saturating_sub(1) as i64;
        (from as i64 + i64::from(d)).clamp(0, last) as usize
    }

    /// Apply `event` and return what the desktop does for it, in order.
    pub fn update(&mut self, event: Event) -> Vec<Command> {
        match event {
            Event::Key(grid) => self.key(grid),
            Event::CurrentChanged(current) => {
                self.current = current;
                Vec::new()
            }
            Event::ListShown { count } => {
                self.count = count;
                if self.current.is_some_and(|i| i >= count) {
                    self.current = None;
                }
                Vec::new()
            }
        }
    }

    fn key(&self, grid: Grid) -> Vec<Command> {
        match grid {
            Grid::Step { dx, dy } => vec![Command::Step { dx, dy }],
            Grid::First if self.count > 0 => vec![Command::Select(0)],
            Grid::Last if self.count > 0 => vec![Command::Select(self.count - 1)],
            Grid::First | Grid::Last => Vec::new(),
            Grid::Open => self.current.map(Command::ShowViewer).into_iter().collect(),
            Grid::Rate(_) | Grid::Turn { .. } | Grid::MarkBorder => {
                if self.current.is_some() {
                    vec![Command::Act(grid)]
                } else {
                    Vec::new()
                }
            }
            Grid::Resize { .. } | Grid::Filters | Grid::Borders => vec![Command::Forward(grid)],
        }
    }
}
