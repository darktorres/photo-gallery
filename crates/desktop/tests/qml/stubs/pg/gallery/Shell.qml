// A stub of the Shell QObject (crates/desktop/src/shell.rs). Only what the viewer and Main call: `viewerStep` clamps a
// step to the list, as pg_client's Input does (`viewer_step`); the list's length is `count`, which a test sets.
// `devMode` and `globalAction` are Main's (its shortcuts and the log view's button).
import QtQuick

QtObject {
    property int count: 0
    // Dev mode is on (the log view's button shows); the window's global actions do nothing here.
    property bool devMode: false

    function globalAction(code) {
    }

    function viewerStep(from, d) {
        return Math.max(0, Math.min(count - 1, from + d))
    }
}
