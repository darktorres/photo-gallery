// A stub of the ResultAnimationPlayer (crates/desktop/src/animation.rs): the search result's original URL it plays, and
// the sink the frames would go to. The preview test checks what YandexPreview does with it, not the decoder.
import QtQuick

QtObject {
    property string url: ""
    // The height the frames are shown at (0 for the file's own size).
    property int height: 0
    // Why the result could not be played ("" when none): its change is the preview's error.
    property string playerError: ""
    function setSink(sink) {
    }
}
