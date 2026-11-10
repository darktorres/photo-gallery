// The window's shortcuts: keys that work wherever the focus is, the grid, the viewer or a text field. Qt matches a
// shortcut before the focused item's key handlers see the key, so the grid's and the viewer's letter keys never take
// them. Each is one Shortcut with its own sequence (from the key map: Qt fires no shortcut whose sequence another one
// shares). What one does is the controller's: Main passes its code on (Shell.globalAction), which decides which panel
// opens or closes, and when a dialog takes the key instead.
pragma ComponentBehavior: Bound

import QtQuick
import pg.gallery

Item {
    id: shortcuts

    // The shortcuts' codes, in the order of pg_client's Global (a Rust test, keymap.rs, checks the order).
    enum GlobalAction { Inbox, Log, DevMode }

    // A shortcut fired: its GlobalAction code.
    signal activated(int code)

    KeyMap {
        id: keymap
    }

    Shortcut {
        sequence: keymap.globalSequence(GlobalShortcuts.Inbox)
        context: Qt.ApplicationShortcut
        autoRepeat: false
        onActivated: shortcuts.activated(GlobalShortcuts.Inbox)
    }
    Shortcut {
        sequence: keymap.globalSequence(GlobalShortcuts.Log)
        context: Qt.ApplicationShortcut
        autoRepeat: false
        onActivated: shortcuts.activated(GlobalShortcuts.Log)
    }
    Shortcut {
        sequence: keymap.globalSequence(GlobalShortcuts.DevMode)
        context: Qt.ApplicationShortcut
        autoRepeat: false
        onActivated: shortcuts.activated(GlobalShortcuts.DevMode)
    }
}
