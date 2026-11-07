// What the stubs of the video source are doing, for the tests to check (tst_media_view.qml): whether a video is playing
// and which file it is. play() and stop() set it, so a test reads the state after the viewer has settled, not the calls
// on the way there. A singleton, so every stub instance in the test writes the same state.
pragma Singleton
import QtQuick

QtObject {
    property bool playing: false
    property int playingFileId: -1

    function reset() {
        playing = false
        playingFileId = -1
    }
}
