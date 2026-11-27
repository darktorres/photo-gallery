// Rating bar: a rating app's symbols, the current rating ringed; clicking one rates and moves on. It also holds the
// find-similar button, the border mark (N) and the turns (R, Shift+R). It asks for each change through a signal; the
// Viewer does it.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

Rectangle {
    id: bar
    Words {
        id: words
    }

    required property bool still
    required property bool hasBorders
    required property bool borderMarked
    required property int rating

    signal rateRequested(int r)
    signal turnRequested(int d)
    signal borderMarkRequested(bool marked)
    signal findSimilarRequested

    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: 12
    width: barRow.implicitWidth + 16
    height: barRow.implicitHeight + 8
    radius: height / 2
    color: Theme.scrimBacking

    // A second click on the same star within the double-click time is the second half of a double click: it is not a second
    // rating (the viewer has moved on to the next picture by then).
    property int lastStar: -1
    Timer {
        id: starGuard
        interval: Application.styleHints.mouseDoubleClickInterval
    }

    RowLayout {
        id: barRow
        anchors.centerIn: parent
        spacing: 4
        // The buttons take the keyboard by Tab only, not by a click: a click must not leave the focus on a button, where
        // Space would press it (on the next picture) instead of the viewer stepping.
        ToolButton {
            text: "≈"
            visible: bar.still
            font.pixelSize: 22
            focusPolicy: Qt.TabFocus
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-find-similar-tooltip")
            onClicked: bar.findSimilarRequested()
        }
        // Borders found: marked as not a border while checked. The checked state is the query's, not the button's own: a
        // refused or failed mark leaves the button as the photo is.
        ToolButton {
            text: "▭"
            visible: bar.hasBorders
            checked: bar.borderMarked
            focusPolicy: Qt.TabFocus
            font.family: Theme.symbolFamily
            font.pixelSize: 22
            font.strikeout: bar.borderMarked
            ToolTip.visible: hovered
            ToolTip.text: bar.borderMarked ? words.text("ui-unmark-not-border") : words.text("ui-not-a-border")
            onClicked: bar.borderMarkRequested(!bar.borderMarked)
        }
        ToolButton {
            text: "⟲"
            font.family: Theme.symbolFamily
            font.pixelSize: 22
            focusPolicy: Qt.TabFocus
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-turn-left")
            onClicked: bar.turnRequested(-1)
        }
        Repeater {
            model: 6
            ToolButton {
                id: star
                required property int index
                text: Ratings.symbol(index)
                // ☆ (no rating) is a symbol; the other ratings are emoji.
                font.family: index === 0 ? Theme.symbolFamily : Theme.emojiFamily
                font.pixelSize: 22
                focusPolicy: Qt.TabFocus
                ToolTip.visible: hovered
                ToolTip.text: words.phrase("ui-stars", index, ("" + (index)), "")
                background: Rectangle {
                    radius: width / 2
                    color: star.hovered ? "#40ffffff" : "transparent"
                    border.color: "#ffd24a"
                    border.width: bar.rating === star.index ? 2 : 0
                }
                onClicked: {
                    if (starGuard.running && bar.lastStar === star.index)
                        return;
                    starGuard.restart();
                    bar.lastStar = star.index;
                    bar.rateRequested(star.index);
                }
            }
        }
        ToolButton {
            text: "⟳"
            font.family: Theme.symbolFamily
            font.pixelSize: 22
            focusPolicy: Qt.TabFocus
            ToolTip.visible: hovered
            ToolTip.text: words.text("ui-turn-right")
            onClicked: bar.turnRequested(1)
        }
    }
}
