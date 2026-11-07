// A stub of the Shell QObject (crates/desktop/src/shell.rs). Only what the viewer calls: `viewerStep` clamps a step to
// the list, as pg_client's Input does (`viewer_step`); the list's length is `count`, which a test sets.
import QtQuick

QtObject {
    property int count: 0

    function viewerStep(from, d) {
        return Math.max(0, Math.min(count - 1, from + d))
    }
}
