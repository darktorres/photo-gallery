// A stub of the AnimationPlayer (crates/desktop/src/animation.rs): the animation's file (its id) and playing state, and the
// sink the frames would go to. The viewer test checks what Viewer and MediaView do with it, not the decoder.
import QtQuick

QtObject {
    property int fileId: 0
    property bool playing: false
    function setSink(sink) {}
}
