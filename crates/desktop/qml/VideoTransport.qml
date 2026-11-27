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

    signal playRequested
    signal muteRequested
    signal volumeRequested(real value)
    // A position dragged to (ms).
    signal seekRequested(real ms)

    Words {
        id: words
    }

    // A playback position: m:ss, or h:mm:ss from an hour.
    function time(ms: real): string {
        return words.timecode(Math.floor(ms / 1000));
    }

    visible: showTransport
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: 12 + barHeight + 8
    width: Math.min(parent.width - 40, 900)
    height: transportRow.implicitHeight + 8
    radius: 8
    color: Theme.scrimBacking

    RowLayout {
        id: transportRow
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 8
        ToolButton {
            // ⏸ with VS16 (U+FE0F), the emoji; ▶ a symbol.
            text: transport.playing ? "⏸️" : "▶"
            font.family: transport.playing ? Theme.emojiFamily : Theme.symbolFamily
            font.pixelSize: 18
            // Not in the focus chain: the transport hides (zen, a still picture), and a control that keeps the focus then
            // takes the arrow keys and Space from the viewer. K plays and pauses from the keys.
            focusPolicy: Qt.NoFocus
            onClicked: transport.playRequested()
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-play-pause")
        }
        Label {
            textFormat: Text.PlainText
            text: transport.time(transport.position)
            color: "#cccccc"
        }
        Slider {
            id: seek
            Layout.fillWidth: true
            from: 0
            to: Math.max(1, transport.duration)
            enabled: transport.seekable
            // See the play button: the arrow keys step the viewer, not this slider.
            focusPolicy: Qt.NoFocus
            onMoved: transport.seekRequested(value)
            // Follows the playback except while dragged.
            Binding on value {
                when: !seek.pressed
                value: transport.position
            }
        }
        Label {
            textFormat: Text.PlainText
            text: transport.time(transport.duration)
            color: "#cccccc"
        }
        ToolButton {
            // Muted, or silent at zero volume: the mute flag decides the icon, so a mute at zero volume shows.
            text: transport.muted ? "🔇" : transport.volume === 0 ? "🔈" : "🔊"
            font.family: Theme.emojiFamily
            font.pixelSize: 16
            focusPolicy: Qt.NoFocus
            onClicked: transport.muteRequested()
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-mute")
        }
        Slider {
            from: 0
            to: 1
            value: transport.volume
            focusPolicy: Qt.NoFocus
            onMoved: transport.volumeRequested(value)
            Layout.preferredWidth: 100
        }
    }
}
