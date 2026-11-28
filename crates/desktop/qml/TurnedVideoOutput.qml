// A video output (or an animation's frames) shown turned as the file's orientation says: mirrored left-right first when
// `mirrored` (Orientation::parts: the mirror, then the quarter turns clockwise). The picture's box is `swapsAxes` when the
// turns swap its sides. `fillMode` is fit (the viewer, the preview) or crop (a tile, as its still is). The players are
// given `output` (the VideoOutput) or its `videoSink`.
import QtQuick
import QtMultimedia

Item {
    id: turned

    required property bool swapsAxes
    required property int turns
    required property bool mirrored
    property int fillMode: VideoOutput.PreserveAspectFit

    readonly property alias output: out
    readonly property alias videoSink: out.videoSink

    Item {
        id: box
        anchors.centerIn: parent
        width: turned.swapsAxes ? parent.height : parent.width
        height: turned.swapsAxes ? parent.width : parent.height
        // QML turns a box and mirrors it in the other order from the file's (it turns, then mirrors). A mirror followed by
        // `t` turns is the mirror then `-t` turns (M·R(t) = R(-t)·M), so a mirrored box takes the inverse turn: the test
        // (tst_turned_video_output.qml) checks the corners of both.
        rotation: (turned.mirrored ? (4 - ((turned.turns % 4) + 4) % 4) % 4 : ((turned.turns % 4) + 4) % 4) * 90
        // The mirror is about the box's centre.
        transform: Scale {
            origin.x: box.width / 2
            origin.y: box.height / 2
            xScale: turned.mirrored ? -1 : 1
        }
        VideoOutput {
            id: out
            anchors.fill: parent
            fillMode: turned.fillMode
        }
    }
}
