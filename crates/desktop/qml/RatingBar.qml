// Rating bar: ImageRater's symbols, the current rating ringed; clicking one rates and moves on. It also holds the
// find-similar button, the border mark (N) and the turns (R, Shift+R). It asks for each change through a signal; the
// Viewer does it.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts

Rectangle {
    id: bar

    required property bool still
    required property bool hasBorders
    required property bool borderMarked
    required property int rating

    signal rateAsked(int r)
    signal turnAsked(int d)
    signal borderMarkAsked(bool marked)
    signal findSimilarAsked()

    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: 12
    width: barRow.implicitWidth + 16
    height: barRow.implicitHeight + 8
    radius: height / 2
    color: "#a0000000"

    RowLayout {
        id: barRow
        anchors.centerIn: parent
        spacing: 4
        ToolButton {
            text: "≈"
            visible: bar.still
            font.pixelSize: 22
            ToolTip.visible: hovered
            ToolTip.text: "Find similar images on Yandex"
            onClicked: bar.findSimilarAsked()
        }
        // Borders found: marked as not a border while checked.
        ToolButton {
            text: "▭"
            visible: bar.hasBorders
            checkable: true
            checked: bar.borderMarked
            font.pixelSize: 22
            font.strikeout: bar.borderMarked
            ToolTip.visible: hovered
            ToolTip.text: bar.borderMarked
                ? "Marked not a border: unmark (N)"
                : "Not a border (N)"
            onClicked: bar.borderMarkAsked(!bar.borderMarked)
        }
        ToolButton {
            text: "⟲"
            font.pixelSize: 22
            ToolTip.visible: hovered
            ToolTip.text: "Turn left (R)"
            onClicked: bar.turnAsked(-1)
        }
        Repeater {
            model: 6
            ToolButton {
                id: star
                required property int index
                text: Ratings.symbol(index)
                font.pixelSize: 22
                ToolTip.visible: hovered
                ToolTip.text: index === 0 ? "Unrated (0)"
                    : index + (index === 1 ? " star" : " stars") + " (" + index + ")"
                background: Rectangle {
                    radius: width / 2
                    color: star.hovered ? "#40ffffff" : "transparent"
                    border.color: "#ffd24a"
                    border.width: bar.rating === star.index ? 2 : 0
                }
                onClicked: bar.rateAsked(star.index)
            }
        }
        ToolButton {
            text: "⟳"
            font.pixelSize: 22
            ToolTip.visible: hovered
            ToolTip.text: "Turn right (Shift+R)"
            onClicked: bar.turnAsked(1)
        }
    }
}
