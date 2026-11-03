//! The key map: what a key does in the grid and in the viewer, as actions the front end carries out, and the hint lines
//! shown with them. The keys are the front end's own (`Key`), not a toolkit's codes: the desktop turns its codes into
//! these. Each hint line is made from the same labels as the map's documentation, so they are written once.

/// A key the front end can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Return and Enter.
    Enter,
    Escape,
    Backspace,
    Space,
    /// `+` and `=`.
    Plus,
    Minus,
    /// `0` to `9`.
    Digit(u8),
    /// A letter, upper case.
    Letter(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

/// A grid action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    /// Ctrl +/−: the rows grow or shrink.
    Resize {
        grow: bool,
    },
    /// The arrows: the selection moves by one cell.
    Step {
        dx: i32,
        dy: i32,
    },
    First,
    Last,
    /// Enter or Space on the selected item: view it.
    Open,
    /// 0–5 on the selected item.
    Rate(u8),
    /// R (back) or Shift+R (forward) on the selected item.
    Turn {
        forward: bool,
    },
    /// F: show or hide the filters.
    Filters,
    /// B: show or hide the borders.
    Borders,
    /// N: mark or unmark the selected item as not a border.
    MarkBorder,
}

/// What the viewer shows, for its key map and its hint line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub has_borders: bool,
    pub still: bool,
    pub video: bool,
}

/// A viewer action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewer {
    Close,
    /// Left and Backspace back, Right and Space on.
    Step {
        forward: bool,
    },
    /// 0–5 rate the shown item; Up rates 4 and Down rates 1.
    Rate(u8),
    Turn {
        forward: bool,
    },
    /// Z: the real size mode, which opens images at 1:1.
    RealSize,
    ZoomIn,
    ZoomOut,
    /// L: the 10× loupe.
    Loupe,
    Borders,
    MarkBorder,
    /// H: hide the overlays.
    Hide,
    /// K: play or pause a video or an animation.
    PlayPause,
    /// M: mute a video.
    Mute,
}

/// The grid's action for a key, given whether an item is selected (the keys that act on it need one).
pub fn grid(key: Key, mods: Mods, has_current: bool) -> Option<Grid> {
    Some(match key {
        Key::Plus if mods.ctrl => Grid::Resize { grow: true },
        Key::Minus if mods.ctrl => Grid::Resize { grow: false },
        Key::Left => Grid::Step { dx: -1, dy: 0 },
        Key::Right => Grid::Step { dx: 1, dy: 0 },
        Key::Up => Grid::Step { dx: 0, dy: -1 },
        Key::Down => Grid::Step { dx: 0, dy: 1 },
        Key::Home => Grid::First,
        Key::End => Grid::Last,
        Key::Enter | Key::Space if has_current => Grid::Open,
        Key::Digit(n) if n <= 5 && has_current => Grid::Rate(n),
        Key::Letter('R') if has_current => Grid::Turn { forward: mods.shift },
        Key::Letter('F') if !mods.ctrl => Grid::Filters,
        Key::Letter('B') => Grid::Borders,
        Key::Letter('N') if has_current => Grid::MarkBorder,
        _ => return None,
    })
}

/// The viewer's action for a key, given what it shows.
pub fn viewer(key: Key, mods: Mods, shown: Shown) -> Option<Viewer> {
    Some(match key {
        Key::Escape | Key::Enter => Viewer::Close,
        Key::Left | Key::Backspace => Viewer::Step { forward: false },
        Key::Right | Key::Space => Viewer::Step { forward: true },
        Key::Up => Viewer::Rate(4),
        Key::Down => Viewer::Rate(1),
        Key::Digit(n) if n <= 5 => Viewer::Rate(n),
        Key::Letter('R') => Viewer::Turn { forward: mods.shift },
        Key::Letter('Z') => Viewer::RealSize,
        Key::Plus => Viewer::ZoomIn,
        Key::Minus => Viewer::ZoomOut,
        Key::Letter('L') => Viewer::Loupe,
        Key::Letter('B') => Viewer::Borders,
        Key::Letter('N') if shown.has_borders => Viewer::MarkBorder,
        Key::Letter('H') => Viewer::Hide,
        Key::Letter('K') if !shown.still => Viewer::PlayPause,
        Key::Letter('M') if shown.video => Viewer::Mute,
        _ => return None,
    })
}

/// A key in the Yandex panels: the background closes; the preview also steps to a neighbour and picks the shown result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Close,
    /// Left and right step through the results.
    Step {
        forward: bool,
    },
    /// Space picks the shown result.
    Pick,
}

/// The panels' action for a key (which panel has the focus decides what it does).
pub fn panel(key: Key) -> Option<Panel> {
    Some(match key {
        Key::Escape => Panel::Close,
        Key::Left => Panel::Step { forward: false },
        Key::Right => Panel::Step { forward: true },
        Key::Space => Panel::Pick,
        _ => return None,
    })
}

/// The kind of item the viewer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Still,
    Animated,
    Video,
}

// The labels the hint lines use (and the map above names).
const SELECT: &str = "Click to select";
const VIEW: &str = "double-click or Enter to view";
const RATE: &str = "0–5 rate";
const TURN: &str = "R / Shift+R turn";
const ARROWS: &str = "arrows move";
const FILTERS: &str = "F filters";
const BORDERS: &str = "B borders";
const NOT_BORDER: &str = "N not a border";
const RESIZE: &str = "Ctrl+wheel or Ctrl +/− resize";
const STEP: &str = "← → step";
const PLAY: &str = "K play/pause";
const MUTE: &str = "M mute";
const UP_4: &str = "↑ 4";
const DOWN_1: &str = "↓ 1";
const TURN_BACK: &str = "R ⟲";
const TURN_ON: &str = "⇧R ⟳";
const WHEEL_ZOOM: &str = "wheel zoom";
const ONE_TO_ONE: &str = "double-click 1:1";
const REAL_SIZE: &str = "Z real size";
const LOUPE: &str = "L loupe";
const HIDE: &str = "H hide";
const CLOSE: &str = "Esc close";

/// The grid's hint line (shown when nothing is selected).
pub fn grid_hint() -> String {
    [SELECT, VIEW, RATE, TURN, ARROWS, FILTERS, BORDERS, NOT_BORDER, RESIZE].join(" · ")
}

/// The viewer's hint line for what it shows.
pub fn viewer_hint(media: Media, shown: Shown) -> String {
    let mut parts = vec![STEP];
    match media {
        Media::Video => parts.extend([PLAY, MUTE]),
        Media::Animated => parts.push(PLAY),
        Media::Still => {}
    }
    parts.extend([RATE, UP_4, DOWN_1, TURN_BACK, TURN_ON]);
    if media == Media::Still {
        parts.extend([WHEEL_ZOOM, ONE_TO_ONE, REAL_SIZE, LOUPE, BORDERS]);
        if shown.has_borders {
            parts.push(NOT_BORDER);
        }
    }
    parts.extend([HIDE, CLOSE]);
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, shift: false };
    const SHIFT: Mods = Mods { ctrl: false, shift: true };
    const STILL: Shown = Shown { has_borders: false, still: true, video: false };

    #[test]
    fn the_arrows_move_the_grid_with_or_without_a_selection() {
        assert_eq!(grid(Key::Left, NONE, false), Some(Grid::Step { dx: -1, dy: 0 }));
        assert_eq!(grid(Key::Down, NONE, true), Some(Grid::Step { dx: 0, dy: 1 }));
    }

    #[test]
    fn keys_that_act_on_the_selection_need_one() {
        assert_eq!(grid(Key::Enter, NONE, false), None);
        assert_eq!(grid(Key::Enter, NONE, true), Some(Grid::Open));
        assert_eq!(grid(Key::Digit(3), NONE, false), None);
        assert_eq!(grid(Key::Digit(3), NONE, true), Some(Grid::Rate(3)));
        assert_eq!(grid(Key::Letter('N'), NONE, false), None);
    }

    #[test]
    fn digits_past_five_are_not_ratings() {
        assert_eq!(grid(Key::Digit(7), NONE, true), None);
        assert_eq!(viewer(Key::Digit(7), NONE, STILL), None);
    }

    #[test]
    fn ctrl_plus_and_minus_resize_the_grid_and_plain_ones_do_nothing() {
        assert_eq!(grid(Key::Plus, CTRL, false), Some(Grid::Resize { grow: true }));
        assert_eq!(grid(Key::Minus, CTRL, false), Some(Grid::Resize { grow: false }));
        assert_eq!(grid(Key::Plus, NONE, false), None);
    }

    #[test]
    fn r_turns_back_or_forward_by_shift() {
        assert_eq!(grid(Key::Letter('R'), NONE, true), Some(Grid::Turn { forward: false }));
        assert_eq!(grid(Key::Letter('R'), SHIFT, true), Some(Grid::Turn { forward: true }));
        assert_eq!(viewer(Key::Letter('R'), SHIFT, STILL), Some(Viewer::Turn { forward: true }));
    }

    #[test]
    fn the_viewer_keys_that_depend_on_the_item() {
        let video = Shown { has_borders: false, still: false, video: true };
        assert_eq!(viewer(Key::Letter('K'), NONE, STILL), None);
        assert_eq!(viewer(Key::Letter('K'), NONE, video), Some(Viewer::PlayPause));
        assert_eq!(viewer(Key::Letter('M'), NONE, STILL), None);
        assert_eq!(viewer(Key::Letter('M'), NONE, video), Some(Viewer::Mute));
        assert_eq!(viewer(Key::Letter('N'), NONE, STILL), None);
        let bordered = Shown { has_borders: true, ..STILL };
        assert_eq!(viewer(Key::Letter('N'), NONE, bordered), Some(Viewer::MarkBorder));
    }

    #[test]
    fn enter_and_escape_close_the_viewer_and_space_steps_on() {
        assert_eq!(viewer(Key::Escape, NONE, STILL), Some(Viewer::Close));
        assert_eq!(viewer(Key::Enter, NONE, STILL), Some(Viewer::Close));
        assert_eq!(viewer(Key::Space, NONE, STILL), Some(Viewer::Step { forward: true }));
        assert_eq!(viewer(Key::Backspace, NONE, STILL), Some(Viewer::Step { forward: false }));
    }

    #[test]
    fn the_grid_hint_names_every_key_it_handles() {
        assert_eq!(
            grid_hint(),
            "Click to select · double-click or Enter to view · 0–5 rate · R / Shift+R turn · arrows move · F filters · \
             B borders · N not a border · Ctrl+wheel or Ctrl +/− resize"
        );
    }

    #[test]
    fn the_panel_keys_close_step_and_pick() {
        assert_eq!(panel(Key::Escape), Some(Panel::Close));
        assert_eq!(panel(Key::Left), Some(Panel::Step { forward: false }));
        assert_eq!(panel(Key::Space), Some(Panel::Pick));
        assert_eq!(panel(Key::Letter('R')), None);
    }

    #[test]
    fn the_viewer_hint_follows_the_media() {
        assert_eq!(
            viewer_hint(Media::Still, STILL),
            "← → step · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · wheel zoom · double-click 1:1 · Z real size · L loupe · \
             B borders · H hide · Esc close"
        );
        let bordered = Shown { has_borders: true, ..STILL };
        assert!(viewer_hint(Media::Still, bordered).contains("· N not a border · H hide"));
        assert_eq!(
            viewer_hint(Media::Video, STILL),
            "← → step · K play/pause · M mute · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · H hide · Esc close"
        );
        assert_eq!(
            viewer_hint(Media::Animated, STILL),
            "← → step · K play/pause · 0–5 rate · ↑ 4 · ↓ 1 · R ⟲ · ⇧R ⟳ · H hide · Esc close"
        );
    }
}
