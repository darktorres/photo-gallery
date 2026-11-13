//! The key map: what a key does in the grid and in the viewer, as actions the front end carries out, and the hint lines
//! shown with them. The keys are the front end's own (`Key`), not a toolkit's codes: the desktop turns its codes into
//! these. Each hint line is made from the same labels as the map's documentation, so they are written once.
use serde::{Deserialize, Serialize};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// A window shortcut: it works wherever the focus is (Qt matches a shortcut before the focused item sees the key), and
/// the controller decides what it does (`Controller`'s rule for the modal panels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Global {
    /// Open or close the inbox.
    Inbox,
    /// Open or close the log view (in dev mode).
    Log,
    /// Turn dev mode on or off.
    DevMode,
}

impl Global {
    /// Every shortcut, in the order of `GlobalAction` in GlobalShortcuts.qml (its codes are these positions).
    pub const ALL: [Global; 3] = [Global::Inbox, Global::Log, Global::DevMode];

    /// Its key sequence, as Qt's `Shortcut.sequence` reads it (one sequence per shortcut: Qt does not fire a sequence two
    /// shortcuts share).
    pub fn sequence(self) -> &'static str {
        match self {
            Global::Inbox => "Ctrl+Shift+N",
            Global::Log => "Ctrl+Shift+L",
            Global::DevMode => "Ctrl+Shift+D",
        }
    }
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

/// A label of a key hint line. Its words are the entry `id` of `locales/en/ui.ftl`; `text::hint` joins them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Select,
    View,
    Rate,
    Turn,
    Arrows,
    Filters,
    Borders,
    NotBorder,
    Resize,
    Step,
    Play,
    Mute,
    Up4,
    Down1,
    TurnBack,
    TurnOn,
    WheelZoom,
    OneToOne,
    RealSize,
    Loupe,
    Hide,
    Close,
}

impl Hint {
    /// Every label.
    pub const ALL: [Hint; 22] = [
        Hint::Select,
        Hint::View,
        Hint::Rate,
        Hint::Turn,
        Hint::Arrows,
        Hint::Filters,
        Hint::Borders,
        Hint::NotBorder,
        Hint::Resize,
        Hint::Step,
        Hint::Play,
        Hint::Mute,
        Hint::Up4,
        Hint::Down1,
        Hint::TurnBack,
        Hint::TurnOn,
        Hint::WheelZoom,
        Hint::OneToOne,
        Hint::RealSize,
        Hint::Loupe,
        Hint::Hide,
        Hint::Close,
    ];

    /// The entry of `locales/en/ui.ftl` that words this label.
    pub fn id(self) -> &'static str {
        match self {
            Hint::Select => "ui-hint-select",
            Hint::View => "ui-hint-view",
            Hint::Rate => "ui-hint-rate",
            Hint::Turn => "ui-hint-turn",
            Hint::Arrows => "ui-hint-arrows",
            Hint::Filters => "ui-hint-filters",
            Hint::Borders => "ui-hint-borders",
            Hint::NotBorder => "ui-hint-not-border",
            Hint::Resize => "ui-hint-resize",
            Hint::Step => "ui-hint-step",
            Hint::Play => "ui-hint-play",
            Hint::Mute => "ui-hint-mute",
            Hint::Up4 => "ui-hint-up-4",
            Hint::Down1 => "ui-hint-down-1",
            Hint::TurnBack => "ui-hint-turn-back",
            Hint::TurnOn => "ui-hint-turn-on",
            Hint::WheelZoom => "ui-hint-wheel-zoom",
            Hint::OneToOne => "ui-hint-one-to-one",
            Hint::RealSize => "ui-hint-real-size",
            Hint::Loupe => "ui-hint-loupe",
            Hint::Hide => "ui-hint-hide",
            Hint::Close => "ui-hint-close",
        }
    }
}

/// The labels of the grid's hint line (shown when nothing is selected).
pub fn grid_hint() -> Vec<Hint> {
    use Hint::*;
    vec![Select, View, Rate, Turn, Arrows, Filters, Borders, NotBorder, Resize]
}

/// The labels of the viewer's hint line for what it shows.
pub fn viewer_hint(media: Media, shown: Shown) -> Vec<Hint> {
    use Hint::*;
    let mut parts = vec![Step];
    match media {
        Media::Video => parts.extend([Play, Mute]),
        Media::Animated => parts.push(Play),
        Media::Still => {}
    }
    parts.extend([Rate, Up4, Down1, TurnBack, TurnOn]);
    if media == Media::Still {
        parts.extend([WheelZoom, OneToOne, RealSize, Loupe, Borders]);
        if shown.has_borders {
            parts.push(NotBorder);
        }
    }
    parts.extend([Hide, Close]);
    parts
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
        use Hint::*;
        assert_eq!(grid_hint(), [Select, View, Rate, Turn, Arrows, Filters, Borders, NotBorder, Resize]);
    }

    #[test]
    fn each_global_shortcut_has_its_own_sequence_with_ctrl_and_shift() {
        let sequences: std::collections::BTreeSet<&str> = Global::ALL.iter().map(|g| g.sequence()).collect();
        assert_eq!(sequences.len(), Global::ALL.len());
        assert!(sequences.iter().all(|s| s.starts_with("Ctrl+Shift+")));
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
        use Hint::*;
        assert_eq!(
            viewer_hint(Media::Still, STILL),
            [Step, Rate, Up4, Down1, TurnBack, TurnOn, WheelZoom, OneToOne, RealSize, Loupe, Borders, Hide, Close]
        );
        let bordered = Shown { has_borders: true, ..STILL };
        assert!(viewer_hint(Media::Still, bordered).windows(2).any(|w| w == [NotBorder, Hide]));
        assert_eq!(
            viewer_hint(Media::Video, STILL),
            [Step, Play, Mute, Rate, Up4, Down1, TurnBack, TurnOn, Hide, Close]
        );
        assert_eq!(viewer_hint(Media::Animated, STILL), [Step, Play, Rate, Up4, Down1, TurnBack, TurnOn, Hide, Close]);
    }
}
