// Everything over the picture (the root hides it in zen mode): the counter, the key hint, the close button, the line
// about the item (its rating, zoom, modes and state) and the rating bar. It asks the root for each action through a
// signal. The key hint comes from the KeyMap facade, the zoom from the Viewport facade.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Item {
    id: hud
    Words {
        id: words
    }

    anchors.fill: parent

    required property int index
    required property int count
    required property int kind
    required property bool hasBorders
    required property bool still
    required property bool zoomable
    required property bool loupe
    // The original could not be loaded.
    required property bool fitFailed
    // Quarter turns asked for but not yet in the file.
    required property int turns
    required property int rating
    required property bool borderMarked
    // A line about the item, from the gallery.
    required property string info
    required property string playerError
    required property Settings settings
    required property Viewport viewport

    // The rating bar's height (the root's transport sits above it).
    readonly property real barHeight: ratingBar.height

    signal closeRequested
    signal rateRequested(int r)
    signal turnRequested(int d)
    signal borderMarkRequested(bool marked)
    signal findSimilarRequested

    KeyMap {
        id: keymap
    }

    Label {
        textFormat: Text.PlainText
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.margins: 10
        color: "#999999"
        style: Text.Outline
        styleColor: "black"
        text: words.count(hud.index + 1) + " / " + words.count(hud.count)
    }
    Label {
        textFormat: Text.PlainText
        anchors.right: closeButton.left
        anchors.verticalCenter: closeButton.verticalCenter
        anchors.rightMargin: 10
        color: "#999999"
        style: Text.Outline
        styleColor: "black"
        text: keymap.viewerHint(hud.kind, hud.hasBorders)
    }
    ToolButton {
        id: closeButton
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 4
        text: "✕"
        onClicked: hud.closeRequested()
    }

    Label {
        textFormat: Text.PlainText
        anchors.left: parent.left
        anchors.right: ratingBar.left
        anchors.verticalCenter: ratingBar.verticalCenter
        anchors.margins: 10
        elide: Text.ElideMiddle
        color: "#cccccc"
        style: Text.Outline
        styleColor: "black"
        text: hud.info + (hud.rating > 0 ? " · " + Ratings.symbol(hud.rating) : "") + (hud.zoomable ? " · " + Math.round(hud.viewport.zoom * 100) + "%" : "") + (hud.settings.realSize ? " · " + words.text("ui-real-size") : "") + (hud.loupe ? " · " + words.text("ui-loupe") : "") + (hud.turns !== 0 ? " · " + words.text("ui-turning") : "") + (hud.fitFailed ? " · " + words.text("ui-original-unavailable") : "") + (hud.kind === GalleryModel.Video && hud.playerError !== "" ? " · " + words.phrase("ui-player-error", 0, hud.playerError, "") : "")
    }

    RatingBar {
        id: ratingBar
        still: hud.still
        hasBorders: hud.hasBorders
        borderMarked: hud.borderMarked
        rating: hud.rating
        onRateRequested: r => hud.rateRequested(r)
        onTurnRequested: d => hud.turnRequested(d)
        onBorderMarkRequested: marked => hud.borderMarkRequested(marked)
        onFindSimilarRequested: hud.findSimilarRequested()
    }
}
