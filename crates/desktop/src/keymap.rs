//! The key map's Qt glue (`pg_client::keys`): Qt's key codes and modifiers become the front end's `Key` and `Mods`. The grid's
//! keys are decided in `Shell` (`grid_key`: the grid's action for a key, which the controller carries out).
//! The viewer's and the Yandex panels' actions come back as codes the QML switches on, with the argument (a rating) from a
//! second call. The codes are the positions in each QML file's `enum` of the same name, so the two lists are kept in the
//! same order: the viewer's and the panels' actions, the window's shortcuts (`GlobalAction` in GlobalShortcuts.qml, with
//! their sequences from here) and the modal panels' kinds (`Kind` in PanelKind.qml). The map and the hint lines are
//! pg-client's.

pub(crate) mod state;

use crate::keymap_bridge::qobject;

use cxx_qt_lib::QString;
use pg_client::controller::panels::PanelKind;
use pg_client::keys::{self, Global, Grid, Key, Media, Mods, Panel, Shown, Viewer};

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
        KEY_0..=KEY_9 => Key::Digit(u8::try_from(code - KEY_0).ok()?),
        KEY_A..=KEY_Z => Key::Letter(char::from(u8::try_from(code).ok()?)),
        _ => return None,
    })
}

fn mods_of(modifiers: i32) -> Mods {
    Mods { ctrl: modifiers & MOD_CTRL != 0, shift: modifiers & MOD_SHIFT != 0 }
}

/// The media a `GalleryModel.MediaKind` value is (QML passes the enum's value as an int): the bridge's enum read by its
/// values, so a reordering of it changes nothing here. An unknown value is a still.
fn media_of(kind: i32) -> Media {
    use crate::gallery_bridge::qobject::MediaKind;
    match kind {
        k if k == MediaKind::Animated.repr => Media::Animated,
        k if k == MediaKind::Video.repr => Media::Video,
        _ => Media::Still,
    }
}

fn shown_of(kind: i32, has_borders: bool) -> Shown {
    let media = media_of(kind);
    Shown { has_borders, still: media == Media::Still, video: media == Media::Video }
}

// The codes: `ViewerAction` in Viewer.qml and `PanelAction` in YandexPanel.qml list these
// names in this order. Each action's argument is its rating (0 for the actions without one).

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

/// The window shortcut of a `GlobalAction` code.
pub(crate) fn global_of(code: i32) -> Option<Global> {
    usize::try_from(code).ok().and_then(|i| Global::ALL.get(i).copied())
}

/// The modal panel of a `PanelKind.Kind` code.
pub(crate) fn panel_kind_of(code: i32) -> Option<PanelKind> {
    usize::try_from(code).ok().and_then(|i| PanelKind::ALL.get(i).copied())
}

/// A modal panel's `PanelKind.Kind` code.
pub(crate) fn panel_kind_code(kind: PanelKind) -> i32 {
    PanelKind::ALL.iter().position(|k| *k == kind).and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

/// The grid's action for a key press, as the key map decides it (`None` when the grid doesn't use the key).
pub(crate) fn grid_key(key: i32, modifiers: i32, has_current: bool) -> Option<Grid> {
    key_of(key).and_then(|k| keys::grid(k, mods_of(modifiers), has_current))
}

/// The viewer's code and argument for a key press; (-1, 0) when the viewer doesn't use it.
fn viewer_of(key: i32, modifiers: i32, kind: i32, has_borders: bool) -> (i32, i32) {
    let shown = shown_of(kind, has_borders);
    key_of(key).and_then(|k| keys::viewer(k, mods_of(modifiers), shown)).map_or((-1, 0), viewer_code)
}

impl qobject::KeyMap {
    pub(crate) fn viewer_action(&self, key: i32, modifiers: i32, kind: i32, has_borders: bool) -> i32 {
        viewer_of(key, modifiers, kind, has_borders).0
    }

    pub(crate) fn viewer_arg(&self, key: i32, modifiers: i32, kind: i32, has_borders: bool) -> i32 {
        viewer_of(key, modifiers, kind, has_borders).1
    }

    pub(crate) fn grid_hint(&self) -> QString {
        QString::from(&keys::grid_hint())
    }

    pub(crate) fn panel_action(&self, key: i32) -> i32 {
        key_of(key).and_then(keys::panel).map_or(-1, panel_code)
    }

    pub(crate) fn viewer_hint(&self, kind: i32, has_borders: bool) -> QString {
        QString::from(&keys::viewer_hint(media_of(kind), shown_of(kind, has_borders)))
    }

    pub(crate) fn global_sequence(&self, code: i32) -> QString {
        global_of(code).map_or_else(QString::default, |g| QString::from(g.sequence()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names of the enum `name` in a QML file, in order: `enum <name> { A, B, … }`.
    #[expect(clippy::string_slice, reason = "find results and an ASCII pattern's length are char boundaries")]
    fn qml_enum(source: &str, name: &str) -> Vec<String> {
        let head = format!("enum {name} {{");
        let rest = &source[source.find(&head).expect("the enum is in the file") + head.len()..];
        let body = &rest[..rest.find('}').expect("the enum closes")];
        body.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    }

    /// The codes are the QML enums' positions, so each action's code must name it in the QML list.
    #[test]
    fn the_viewer_codes_are_the_qml_enum() {
        let expected = [
            ("Close", Viewer::Close),
            ("StepBack", Viewer::Step { forward: false }),
            ("StepForward", Viewer::Step { forward: true }),
            ("Rate", Viewer::Rate(0)),
            ("TurnForward", Viewer::Turn { forward: true }),
            ("TurnBack", Viewer::Turn { forward: false }),
            ("RealSize", Viewer::RealSize),
            ("ZoomIn", Viewer::ZoomIn),
            ("ZoomOut", Viewer::ZoomOut),
            ("Loupe", Viewer::Loupe),
            ("Borders", Viewer::Borders),
            ("MarkBorder", Viewer::MarkBorder),
            ("Hide", Viewer::Hide),
            ("PlayPause", Viewer::PlayPause),
            ("Mute", Viewer::Mute),
        ];
        let names = qml_enum(include_str!("../qml/Viewer.qml"), "ViewerAction");
        assert_eq!(names, expected.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>());
        for (name, action) in expected {
            assert_eq!(names[usize::try_from(viewer_code(action).0).unwrap()], name);
        }
    }

    #[test]
    fn the_panel_codes_are_the_qml_enums() {
        let expected = [
            ("Close", Panel::Close),
            ("StepBack", Panel::Step { forward: false }),
            ("StepForward", Panel::Step { forward: true }),
            ("Pick", Panel::Pick),
        ];
        for source in [include_str!("../qml/YandexPanel.qml"), include_str!("../qml/YandexPreview.qml")] {
            let names = qml_enum(source, "PanelAction");
            assert_eq!(names, expected.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>());
            for (name, action) in expected {
                assert_eq!(names[usize::try_from(panel_code(action)).unwrap()], name);
            }
        }
    }

    #[test]
    fn the_global_codes_are_the_qml_enum() {
        let names = qml_enum(include_str!("../qml/GlobalShortcuts.qml"), "GlobalAction");
        assert_eq!(names, Global::ALL.iter().map(|g| format!("{g:?}")).collect::<Vec<_>>());
        for (i, global) in Global::ALL.into_iter().enumerate() {
            assert_eq!(global_of(i32::try_from(i).unwrap()), Some(global));
        }
        assert_eq!(global_of(-1), None);
        assert_eq!(global_of(i32::try_from(Global::ALL.len()).unwrap()), None);
    }

    #[test]
    fn the_panel_kind_codes_are_the_qml_enum() {
        let names = qml_enum(include_str!("../qml/PanelKind.qml"), "Kind");
        assert_eq!(names, PanelKind::ALL.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>());
        for kind in PanelKind::ALL {
            assert_eq!(panel_kind_of(panel_kind_code(kind)), Some(kind));
        }
        assert_eq!(panel_kind_of(-1), None);
    }

    #[test]
    fn a_media_kind_value_is_its_media() {
        use crate::gallery_bridge::qobject::MediaKind;
        assert_eq!(media_of(MediaKind::Image.repr), Media::Still);
        assert_eq!(media_of(MediaKind::Animated.repr), Media::Animated);
        assert_eq!(media_of(MediaKind::Video.repr), Media::Video);
    }
}
