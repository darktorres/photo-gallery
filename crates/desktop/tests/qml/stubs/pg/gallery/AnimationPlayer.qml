// A stub of the AnimationPlayer (crates/desktop/src/animation.rs): the animation's file (its id) and playing state, and the
// sink the frames would go to. The viewer test checks what Viewer and MediaView do with it, not the decoder.
import QtQuick

QtObject {
    property int fileId: 0
    // The height the frames are shown at (0 for the file's own size).
    property int height: 0
    property bool playing: false
    // Why the animation could not be read or decoded ("" when none): its change is the window's playerError.
    property string playerError: ""
    function setSink(sink) {
    }
    // The kind of the player's error, 0 for none (the stub has no player to ask).
    function playerErrorKind(player: var): int {
        return 0;
    }
}
