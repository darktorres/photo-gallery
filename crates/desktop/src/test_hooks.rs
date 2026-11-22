//! The test hooks as the window runs them: the command line's hook options, read into the steps they ask for, in the order
//! they are run. The shell runs the steps that are its own (the selection, the grid's keys, the similar search); the rest
//! are the viewer's and the panels', which TestHooks.qml performs by the `Kind` it is given (its enum, in this order).

use crate::command_line::Hooks;
use crate::keymap::{KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_UP};

/// What one hook asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hook {
    /// Select this item (`--select`).
    Select(i32),
    /// A grid key, as a Qt key code (`--grid-keys`).
    GridKey(i32),
    /// The similar-image search of the command line's photo (`--find-similar`).
    FindSimilar,
    /// Something the window performs: its kind, and the index, value or text it takes (0, 0 and "" when it takes none).
    Window { kind: Kind, index: i32, value: f64, text: String },
}

/// What the window performs for a `Hook::Window`, in the order of TestHooks.qml's `Kind` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    ViewerShow,
    ViewerLoupe,
    Borders,
    BorderMark,
    ViewerZen,
    ViewerPlay,
    ZoomIn,
    ZoomOne,
    ZoomTo,
    Turn,
    Rate,
    ViewsMenu,
    LibraryPanel,
    SaveView,
    LogPanel,
    InboxExpand,
    InboxPanel,
    AddFolder,
    SaveCrops,
}

impl Kind {
    /// The number TestHooks.qml's `Kind` enum gives this kind.
    pub(crate) const fn code(self) -> i32 {
        match self {
            Self::ViewerShow => 0,
            Self::ViewerLoupe => 1,
            Self::Borders => 2,
            Self::BorderMark => 3,
            Self::ViewerZen => 4,
            Self::ViewerPlay => 5,
            Self::ZoomIn => 6,
            Self::ZoomOne => 7,
            Self::ZoomTo => 8,
            Self::Turn => 9,
            Self::Rate => 10,
            Self::ViewsMenu => 11,
            Self::LibraryPanel => 12,
            Self::SaveView => 13,
            Self::LogPanel => 14,
            Self::InboxExpand => 15,
            Self::InboxPanel => 16,
            Self::AddFolder => 17,
            Self::SaveCrops => 18,
        }
    }
}

fn window(kind: Kind, index: i32, value: f64, text: &str) -> Hook {
    Hook::Window { kind, index, value, text: text.to_owned() }
}

fn plain(kind: Kind, index: i32) -> Option<Hook> {
    Some(window(kind, index, 0.0, ""))
}

/// The hooks that run once the first list is shown, in order. `find_similar` is whether `--find-similar` is given.
pub(crate) fn before_list(hooks: &Hooks, find_similar: bool) -> Vec<Hook> {
    let mut steps = Vec::new();
    if let Some(item) = hooks.select.and_then(|i| i32::try_from(i).ok()) {
        steps.push(Hook::Select(item));
    }
    steps.extend(hooks.grid_keys.iter().filter_map(|key| grid_key(key)));
    if let Some(item) = hooks.view.and_then(|i| i32::try_from(i).ok()) {
        steps.push(window(Kind::ViewerShow, item, 0.0, ""));
        steps.extend(hooks.viewer_keys.iter().filter_map(|key| viewer_key(key)));
        // The zoom comes after the keys, so a `+` or `1` before it is overridden by the zoom given.
        if let Some(zoom) = hooks.zoom {
            steps.push(window(Kind::ZoomTo, 0, zoom, ""));
        }
    }
    if hooks.menu_views {
        steps.push(window(Kind::ViewsMenu, 0, 0.0, ""));
    }
    if hooks.library {
        steps.push(window(Kind::LibraryPanel, 0, 0.0, ""));
    }
    if hooks.save_view {
        steps.push(window(Kind::SaveView, 0, 0.0, ""));
    }
    if find_similar {
        steps.push(Hook::FindSimilar);
    }
    if hooks.log {
        steps.push(window(Kind::LogPanel, 0, 0.0, ""));
    }
    // Before the inbox opens, so it opens at the entry.
    if let Some(code) = hooks.inbox_expand.as_deref().filter(|code| !code.is_empty()) {
        steps.push(window(Kind::InboxExpand, 0, 0.0, code));
    }
    if hooks.inbox {
        steps.push(window(Kind::InboxPanel, 0, 0.0, ""));
    }
    steps
}

/// The hooks that wait for the library job, run once it has started. `add_folder` and `save_crops` say whether
/// `--add-folder` and `--save-crops` are given.
pub(crate) fn after_library_start(add_folder: bool, save_crops: bool) -> Vec<Hook> {
    let mut steps = Vec::new();
    if add_folder {
        steps.push(window(Kind::AddFolder, 0, 0.0, ""));
    }
    if save_crops {
        steps.push(window(Kind::SaveCrops, 0, 0.0, ""));
    }
    steps
}

/// A grid key (`--grid-keys`): L, R, U or D, as the arrow key it stands for.
fn grid_key(key: &str) -> Option<Hook> {
    match key {
        "L" => Some(Hook::GridKey(KEY_LEFT)),
        "R" => Some(Hook::GridKey(KEY_RIGHT)),
        "U" => Some(Hook::GridKey(KEY_UP)),
        "D" => Some(Hook::GridKey(KEY_DOWN)),
        _ => None,
    }
}

/// A viewer key (`--viewer-keys`): the action the user's key makes in the viewer. `N` marks the viewer's own picture
/// (index -1: the window's current one).
fn viewer_key(key: &str) -> Option<Hook> {
    match key {
        "L" => plain(Kind::ViewerLoupe, 0),
        "B" => plain(Kind::Borders, 0),
        "N" => plain(Kind::BorderMark, -1),
        "H" => plain(Kind::ViewerZen, 0),
        "K" => plain(Kind::ViewerPlay, 0),
        "+" => plain(Kind::ZoomIn, 0),
        "1" => plain(Kind::ZoomOne, 0),
        "<" => plain(Kind::Turn, -1),
        ">" => plain(Kind::Turn, 1),
        "r0" => plain(Kind::Rate, 0),
        "r1" => plain(Kind::Rate, 1),
        "r2" => plain(Kind::Rate, 2),
        "r3" => plain(Kind::Rate, 3),
        "r4" => plain(Kind::Rate, 4),
        "r5" => plain(Kind::Rate, 5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_and_grid_keys_come_first_and_the_view_next() {
        let hooks =
            Hooks { select: Some(2), grid_keys: vec!["L".into(), "D".into()], view: Some(5), ..Hooks::default() };
        assert_eq!(
            before_list(&hooks, false),
            vec![
                Hook::Select(2),
                Hook::GridKey(KEY_LEFT),
                Hook::GridKey(KEY_DOWN),
                window(Kind::ViewerShow, 5, 0.0, ""),
            ]
        );
    }

    #[test]
    fn the_viewer_keys_follow_the_view_and_a_given_zoom_comes_after_them() {
        let hooks = Hooks {
            view: Some(1),
            viewer_keys: vec!["+".into(), "r4".into(), "<".into(), "N".into(), "Q".into()],
            zoom: Some(2.0),
            ..Hooks::default()
        };
        assert_eq!(
            before_list(&hooks, false),
            vec![
                window(Kind::ViewerShow, 1, 0.0, ""),
                window(Kind::ZoomIn, 0, 0.0, ""),
                window(Kind::Rate, 4, 0.0, ""),
                window(Kind::Turn, -1, 0.0, ""),
                window(Kind::BorderMark, -1, 0.0, ""),
                window(Kind::ZoomTo, 0, 2.0, ""),
            ]
        );
    }

    #[test]
    fn the_panels_run_in_their_order_with_the_similar_search_between_the_views_and_the_log() {
        let hooks = Hooks {
            menu_views: true,
            library: true,
            save_view: true,
            log: true,
            inbox_expand: Some("scan-folders".into()),
            inbox: true,
            ..Hooks::default()
        };
        assert_eq!(
            before_list(&hooks, true),
            vec![
                window(Kind::ViewsMenu, 0, 0.0, ""),
                window(Kind::LibraryPanel, 0, 0.0, ""),
                window(Kind::SaveView, 0, 0.0, ""),
                Hook::FindSimilar,
                window(Kind::LogPanel, 0, 0.0, ""),
                window(Kind::InboxExpand, 0, 0.0, "scan-folders"),
                window(Kind::InboxPanel, 0, 0.0, ""),
            ]
        );
    }

    #[test]
    fn an_empty_inbox_expand_is_no_hook() {
        let hooks = Hooks { inbox_expand: Some(String::new()), ..Hooks::default() };
        assert!(before_list(&hooks, false).is_empty());
    }

    #[test]
    fn the_library_hooks_are_the_folder_and_the_crops_in_that_order() {
        assert_eq!(
            after_library_start(true, true),
            vec![window(Kind::AddFolder, 0, 0.0, ""), window(Kind::SaveCrops, 0, 0.0, "")]
        );
        assert!(after_library_start(false, false).is_empty());
    }

    #[test]
    fn the_kinds_have_the_window_enum_codes_in_order() {
        let kinds = [
            Kind::ViewerShow,
            Kind::ViewerLoupe,
            Kind::Borders,
            Kind::BorderMark,
            Kind::ViewerZen,
            Kind::ViewerPlay,
            Kind::ZoomIn,
            Kind::ZoomOne,
            Kind::ZoomTo,
            Kind::Turn,
            Kind::Rate,
            Kind::ViewsMenu,
            Kind::LibraryPanel,
            Kind::SaveView,
            Kind::LogPanel,
            Kind::InboxExpand,
            Kind::InboxPanel,
            Kind::AddFolder,
            Kind::SaveCrops,
        ];
        for (position, kind) in kinds.iter().enumerate() {
            assert_eq!(kind.code(), i32::try_from(position).expect("a short list"));
        }
    }
}
