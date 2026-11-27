// Everything over the picture (the root hides it in zen mode): the counter, the key hint, the close button, the line
// about the item (its rating, zoom, modes and state) and the rating bar. It asks the root for each action through a
// signal. The key hint comes from the KeyMap facade, the zoom from the Viewport facade.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
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
    // The picture could not be shown: the fit, the full-size original (1:1, the loupe) or a zoomed region of it failed.
    required property bool fitFailed
    required property bool fullFailed
    required property bool regionFailed
    // Why the fit failed, from the window (Rust's FitError codes): 0 none or the original unavailable, 2 undecodable,
    // 3 rewriting (the file was being written past the wait).
    required property int fitError
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
    signal borderMarkRequested
    signal findSimilarRequested

    readonly property bool pictureFailed: fitFailed || fullFailed || regionFailed

    // The line's words for a failed picture: the fit's own cause when the fit failed, else the original (1:1, the loupe)
    // is unavailable, else a zoomed part of it could not be shown.
    function failureText(): string {
        if (hud.fitFailed) {
            switch (hud.fitError) {
            case 2:
                return words.text("ui-fit-undecodable");
            case 3:
                return words.text("ui-fit-rewriting");
            default:
                return words.text("ui-original-unavailable");
            }
        }
        if (hud.fullFailed)
            return words.text("ui-original-unavailable");
        return words.text("ui-region-failed");
    }

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
        // The hint names the turns by their symbols (⟲ ⟳).
        font.family: Theme.symbolFamily
    }
    ToolButton {
        id: closeButton
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 4
        text: "✕"
        font.family: Theme.symbolFamily
        onClicked: hud.closeRequested()
    }

    // The line's text, in three parts: the item, its rating, then its zoom, modes and state. The rating is an emoji, alone
    // in the emoji font (Theme.emojiFamily); each other part keeps the window's font and takes no more than its width.
    component InfoText: Label {
        textFormat: Text.PlainText
        color: "#cccccc"
        style: Text.Outline
        styleColor: "black"
        Layout.fillWidth: true
        Layout.maximumWidth: implicitWidth
    }
    RowLayout {
        anchors.left: parent.left
        anchors.right: ratingBar.left
        anchors.verticalCenter: ratingBar.verticalCenter
        anchors.margins: 10
        spacing: 0
        InfoText {
            elide: Text.ElideMiddle
            text: hud.info + (hud.rating > 0 ? " · " : "")
        }
        InfoText {
            visible: hud.rating > 0
            text: hud.rating > 0 ? Ratings.symbol(hud.rating) : ""
            font.family: Theme.emojiFamily
            Layout.fillWidth: false
        }
        InfoText {
            elide: Text.ElideRight
            // Real size is a still's mode: a video or an animation does not take it, so the line does not say it.
            text: (hud.zoomable ? " · " + Math.round(hud.viewport.zoom * 100) + "%" : "") + (hud.still && hud.settings.realSize ? " · " + words.text("ui-real-size") : "") + (hud.loupe ? " · " + words.text("ui-loupe") : "") + (hud.turns !== 0 ? " · " + words.text("ui-turning") : "") + (hud.pictureFailed ? " · " + hud.failureText() : "") + (hud.playerError !== "" ? " · " + words.phrase("ui-player-error", 0, hud.playerError, "") : "")
        }
        // The parts keep to the left.
        Item {
            Layout.fillWidth: true
        }
    }

    RatingBar {
        id: ratingBar
        still: hud.still
        hasBorders: hud.hasBorders
        borderMarked: hud.borderMarked
        rating: hud.rating
        onRateRequested: r => hud.rateRequested(r)
        onTurnRequested: d => hud.turnRequested(d)
        onBorderMarkRequested: hud.borderMarkRequested()
        onFindSimilarRequested: hud.findSimilarRequested()
    }
}
