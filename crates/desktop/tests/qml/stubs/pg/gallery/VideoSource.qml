// A stub of the VideoSource (crates/desktop/src/video_source.rs): the video's file played on a player. The viewer test
// checks what MediaView asks of it, not the player.
import QtQuick

QtObject {
    function attach(player) {}
    function play(fileId) {}
    function stop() {}
}
