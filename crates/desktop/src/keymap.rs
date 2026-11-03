//! The key map's Qt glue (`pg_client::keys`): Qt's key codes and modifiers become the front end's `Key` and `Mods`, and the
//! action comes back as a code the QML switches on, with its argument (a rating) from a second call. The codes are the
//! positions in each QML file's `enum` of the same name, so the two lists are kept in the same order. The map and the
//! hint lines are pg-client's.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type KeyMap = super::KeyMapRust;
    }

    unsafe extern "RustQt" {
        /// The grid's action for a key press (Qt's key code and modifiers): a `GridAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "gridAction"]
        fn grid_action(self: &KeyMap, key: i32, modifiers: i32, has_current: bool) -> i32;

        /// The argument of the grid's action (the rating of `GridAction.Rate`), 0 for the others.
        #[qinvokable]
        #[cxx_name = "gridArg"]
        fn grid_arg(self: &KeyMap, key: i32, modifiers: i32, has_current: bool) -> i32;

        /// The viewer's action for a key press, for what it shows (`kind`: "image", "animated" or "video"): a
        /// `ViewerAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "viewerAction"]
        fn viewer_action(self: &KeyMap, key: i32, modifiers: i32, kind: &QString, has_borders: bool) -> i32;

        /// The argument of the viewer's action (the rating of `ViewerAction.Rate`), 0 for the others.
        #[qinvokable]
        #[cxx_name = "viewerArg"]
        fn viewer_arg(self: &KeyMap, key: i32, modifiers: i32, kind: &QString, has_borders: bool) -> i32;

        #[qinvokable]
        #[cxx_name = "gridHint"]
        fn grid_hint(self: &KeyMap) -> QString;

        #[qinvokable]
        #[cxx_name = "viewerHint"]
        fn viewer_hint(self: &KeyMap, kind: &QString, has_borders: bool) -> QString;

        /// The Yandex panels' action for a key press: a `PanelAction` code, or -1 for none.
        #[qinvokable]
        #[cxx_name = "panelAction"]
        fn panel_action(self: &KeyMap, key: i32) -> i32;
    }
}

use cxx_qt_lib::QString;
use pg_client::keys::{self, Grid, Key, Media, Mods, Panel, Shown, Viewer};

#[derive(Default)]
pub struct KeyMapRust;

// Qt's key codes (`Qt::Key`) and modifier flags (`Qt::KeyboardModifier`): the values of Qt's stable ABI.
const KEY_ESCAPE: i32 = 0x0100_0000;
const KEY_BACKSPACE: i32 = 0x0100_0003;
const KEY_RETURN: i32 = 0x0100_0004;
const KEY_ENTER: i32 = 0x0100_0005;
const KEY_HOME: i32 = 0x0100_0010;
const KEY_END: i32 = 0x0100_0011;
const KEY_LEFT: i32 = 0x0100_0012;
const KEY_UP: i32 = 0x0100_0013;
const KEY_RIGHT: i32 = 0x0100_0014;
const KEY_DOWN: i32 = 0x0100_0015;
const KEY_SPACE: i32 = 0x20;
const KEY_PLUS: i32 = 0x2B;
const KEY_MINUS: i32 = 0x2D;
const KEY_0: i32 = 0x30;
const KEY_9: i32 = 0x39;
const KEY_EQUAL: i32 = 0x3D;
const KEY_A: i32 = 0x41;
const KEY_Z: i32 = 0x5A;
const MOD_SHIFT: i32 = 0x0200_0000;
const MOD_CTRL: i32 = 0x0400_0000;

/// The front end's key for a Qt key code, if the map has one.
fn key_of(code: i32) -> Option<Key> {
    Some(match code {
        KEY_ESCAPE => Key::Escape,
        KEY_BACKSPACE => Key::Backspace,
        KEY_RETURN | KEY_ENTER => Key::Enter,
        KEY_HOME => Key::Home,
        KEY_END => Key::End,
        KEY_LEFT => Key::Left,
        KEY_UP => Key::Up,
        KEY_RIGHT => Key::Right,
        KEY_DOWN => Key::Down,
        KEY_SPACE => Key::Space,
        KEY_PLUS | KEY_EQUAL => Key::Plus,
        KEY_MINUS => Key::Minus,
        KEY_0..=KEY_9 => Key::Digit((code - KEY_0) as u8),
        KEY_A..=KEY_Z => Key::Letter(char::from(code as u8)),
        _ => return None,
    })
}

fn mods_of(modifiers: i32) -> Mods {
    Mods { ctrl: modifiers & MOD_CTRL != 0, shift: modifiers & MOD_SHIFT != 0 }
}

fn media_of(kind: &QString) -> Media {
    match kind.to_string().as_str() {
        "video" => Media::Video,
        "animated" => Media::Animated,
        _ => Media::Still,
    }
}

fn shown_of(kind: &QString, has_borders: bool) -> Shown {
    let media = media_of(kind);
    Shown { has_borders, still: media == Media::Still, video: media == Media::Video }
}

// The codes: `GridAction` in Main.qml, `ViewerAction` in Viewer.qml and `PanelAction` in YandexPanel.qml list these
// names in this order. Each action's argument is its rating (0 for the actions without one).

/// The grid's action as its code, with the argument.
fn grid_code(action: Grid) -> (i32, i32) {
    match action {
        Grid::Resize { grow } => (if grow { 0 } else { 1 }, 0),
        Grid::Step { dx, dy } => (
            if dx < 0 {
                2
            } else if dx > 0 {
                3
            } else if dy < 0 {
                4
            } else {
                5
            },
            0,
        ),
        Grid::First => (6, 0),
        Grid::Last => (7, 0),
        Grid::Open => (8, 0),
        Grid::Rate(n) => (9, i32::from(n)),
        Grid::Turn { forward } => (if forward { 10 } else { 11 }, 0),
        Grid::Filters => (12, 0),
        Grid::Borders => (13, 0),
        Grid::MarkBorder => (14, 0),
    }
}

/// The Yandex panels' action as its code.
fn panel_code(action: Panel) -> i32 {
    match action {
        Panel::Close => 0,
        Panel::Step { forward } => {
            if forward {
                2
            } else {
                1
            }
        }
        Panel::Pick => 3,
    }
}

/// The viewer's action as its code, with the argument.
fn viewer_code(action: Viewer) -> (i32, i32) {
    match action {
        Viewer::Close => (0, 0),
        Viewer::Step { forward } => (if forward { 2 } else { 1 }, 0),
        Viewer::Rate(n) => (3, i32::from(n)),
        Viewer::Turn { forward } => (if forward { 4 } else { 5 }, 0),
        Viewer::RealSize => (6, 0),
        Viewer::ZoomIn => (7, 0),
        Viewer::ZoomOut => (8, 0),
        Viewer::Loupe => (9, 0),
        Viewer::Borders => (10, 0),
        Viewer::MarkBorder => (11, 0),
        Viewer::Hide => (12, 0),
        Viewer::PlayPause => (13, 0),
        Viewer::Mute => (14, 0),
    }
}

/// The grid's code and argument for a key press; (-1, 0) when the grid doesn't use it.
fn grid_of(key: i32, modifiers: i32, has_current: bool) -> (i32, i32) {
    key_of(key).and_then(|k| keys::grid(k, mods_of(modifiers), has_current)).map_or((-1, 0), grid_code)
}

/// The viewer's code and argument for a key press; (-1, 0) when the viewer doesn't use it.
fn viewer_of(key: i32, modifiers: i32, kind: &QString, has_borders: bool) -> (i32, i32) {
    let shown = shown_of(kind, has_borders);
    key_of(key).and_then(|k| keys::viewer(k, mods_of(modifiers), shown)).map_or((-1, 0), viewer_code)
}

impl qobject::KeyMap {
    fn grid_action(&self, key: i32, modifiers: i32, has_current: bool) -> i32 {
        grid_of(key, modifiers, has_current).0
    }

    fn grid_arg(&self, key: i32, modifiers: i32, has_current: bool) -> i32 {
        grid_of(key, modifiers, has_current).1
    }

    fn viewer_action(&self, key: i32, modifiers: i32, kind: &QString, has_borders: bool) -> i32 {
        viewer_of(key, modifiers, kind, has_borders).0
    }

    fn viewer_arg(&self, key: i32, modifiers: i32, kind: &QString, has_borders: bool) -> i32 {
        viewer_of(key, modifiers, kind, has_borders).1
    }

    fn grid_hint(&self) -> QString {
        QString::from(&keys::grid_hint())
    }

    fn panel_action(&self, key: i32) -> i32 {
        key_of(key).and_then(keys::panel).map_or(-1, panel_code)
    }

    fn viewer_hint(&self, kind: &QString, has_borders: bool) -> QString {
        QString::from(&keys::viewer_hint(media_of(kind), shown_of(kind, has_borders)))
    }
}
