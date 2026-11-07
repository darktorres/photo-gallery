// A stub of the VideoSource (crates/desktop/src/video_source.rs): the video's file played on a player. The tests check
// what MediaView leaves playing (through VideoLog), not the player.
import QtQuick

QtObject {
    function attach(player) {}
    function play(fileId) {
        VideoLog.playing = true
        VideoLog.playingFileId = fileId
    }
    function stop() {
        VideoLog.playing = false
    }
}
