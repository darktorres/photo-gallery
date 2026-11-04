// Video transport: play/pause, position (drag to seek), sound; just above the rating bar, which is not a sibling to
// anchor to (both are 12 px plus their height from the bottom). It reads the time through its own Words facade.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

Rectangle {
    id: transport

    // The player's state (MediaView's player): whether it plays, its position and duration (ms), and whether it seeks.
    required property bool playing
    required property real position
    required property real duration
    required property bool seekable
    required property bool showTransport
    // The rating bar's height (the transport sits above it).
    required property real barHeight
    required property bool muted
    required property real volume

    signal playAsked()
    signal muteAsked()
    signal volumeAsked(real value)
    // A position dragged to (ms).
    signal seekAsked(real ms)

    Words {
        id: words
    }

    // A playback position: m:ss, or h:mm:ss from an hour.
    function time(ms: real): string {
        return words.timecode(Math.floor(ms / 1000))
    }

    visible: showTransport
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: 12 + barHeight + 8
    width: Math.min(parent.width - 40, 900)
    height: transportRow.implicitHeight + 8
    radius: 8
    color: "#a0000000"

    RowLayout {
        id: transportRow
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 8
        ToolButton {
            text: transport.playing ? "⏸" : "▶"
            font.pixelSize: 18
            onClicked: transport.playAsked()
            ToolTip.visible: hovered
            ToolTip.text: "Play / pause (K)"
        }
        Label { text: transport.time(transport.position); color: "#cccccc" }
        Slider {
            id: seek
            Layout.fillWidth: true
            from: 0
            to: Math.max(1, transport.duration)
            enabled: transport.seekable
            onMoved: transport.seekAsked(value)
            // Follows the playback except while dragged.
            Binding on value {
                when: !seek.pressed
                value: transport.position
            }
        }
        Label { text: transport.time(transport.duration); color: "#cccccc" }
        ToolButton {
            text: transport.muted || transport.volume === 0 ? "🔇" : "🔊"
            font.pixelSize: 16
            onClicked: transport.muteAsked()
            ToolTip.visible: hovered
            ToolTip.text: "Mute (M)"
        }
        Slider {
            from: 0
            to: 1
            value: transport.volume
            onMoved: transport.volumeAsked(value)
            Layout.preferredWidth: 100
        }
    }
}
