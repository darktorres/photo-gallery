// A stub of the KeyMap facade (pg_client's keys): the viewer's action codes, in the order of Viewer.ViewerAction
// (Close 0, StepBack 1, StepForward 2, Rate 3). The real map is pg_client's keys, read through keymap.rs.
import QtQuick

QtObject {
    function viewerAction(key, modifiers, kind, hasBorders) {
        if (key === Qt.Key_Escape) return 0
        if (key === Qt.Key_Left) return 1
        if (key === Qt.Key_Right) return 2
        if (key >= Qt.Key_0 && key <= Qt.Key_5) return 3
        return -1
    }
    function viewerArg(key, modifiers, kind, hasBorders) {
        return key >= Qt.Key_0 && key <= Qt.Key_5 ? key - Qt.Key_0 : 0
    }
    // The hint texts and the Yandex panels' action: the real ones come from keymap.rs. No test reads their values, so
    // the stub gives an empty hint and no action, and the QML that calls them runs without a TypeError.
    function gridHint() { return "" }
    function viewerHint(kind, hasBorders) { return "" }
    function panelAction(key) { return -1 }
    // The window's shortcuts' sequences, as pg_client's Global has them (GlobalShortcuts.GlobalAction's order).
    function globalSequence(code) { return ["Ctrl+Shift+N", "Ctrl+Shift+L", "Ctrl+Shift+D"][code] || "" }
}
