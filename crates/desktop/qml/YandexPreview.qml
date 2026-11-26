// One Yandex result at full size (a result tile's double-click), loaded from its URL: fitted to the window, enlarged when
// it is smaller, or 1:1 (one file pixel per screen pixel) by a click on it or the button. ←/→ step through all the
// results, Space picks or unpicks (`pickRequested`), Escape goes back to the list (`closed`). The panel owns the search:
// it passes the URL, the size text and the pick state of the item shown.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtQuick.Window
import pg.gallery

Rectangle {
    id: preview
    Words {
        id: words
    }

    // The result shown (its index in the search), -1 when none.
    property int item: -1
    property bool actual: false

    // The results: the other sizes first, then the similar images.
    // The window's pixel ratio: the full picture's logical size is its pixels over it.
    required property real dpr
    required property int otherCount
    required property int similarCount
    // The item shown: its picture's URL (`""` while hidden), its file's size as text (`""` until the file is in), and
    // whether it is picked.
    required property string imageUrl
    required property string sizeText
    required property bool picked
    // The pictures of the previous and the next result ("" for none), loaded hidden so a step shows its picture at once.
    required property string prevUrl
    required property string nextUrl

    // The user asked to pick or unpick an item (`-1` for none).
    signal pickRequested(int item)
    // Back to the list.
    signal closed

    readonly property int total: preview.otherCount + preview.similarCount

    // The keys: the key map (pg-client's keys) names the action.
    KeyMap {
        id: keymap
    }

    function open(i: int) {
        preview.item = i;
        preview.actual = false;
        wheelSteps.reset();
        preview.visible = true;
        preview.forceActiveFocus();
    }

    function close() {
        preview.visible = false;
        preview.item = -1;
        preview.closed();
    }

    // The previous (-1) or next (1) result: the search wraps round its results, and says which one is shown now.
    signal stepRequested(int by)
    function step(by: int) {
        preview.stepRequested(by);
        preview.actual = false;
    }

    // 1:1 opens on the image's middle; fitted, the content is the view, scrolled to its start. Done once the
    // content has its new size, on the next event-loop pass.
    Timer {
        id: centre
        interval: 0
        onTriggered: {
            previewView.contentX = Math.max(0, (previewView.contentWidth - previewView.width) / 2);
            previewView.contentY = Math.max(0, (previewView.contentHeight - previewView.height) / 2);
        }
    }
    onActualChanged: centre.restart()

    anchors.fill: parent
    visible: false
    color: Theme.previewFill
    Keys.onPressed: e => {
        const action = keymap.panelAction(e.key);
        if (action < 0)
            return;
        switch (action) {
        case YandexPanel.PanelAction.Close:
            preview.close();
            break;
        case YandexPanel.PanelAction.StepBack:
            preview.step(-1);
            break;
        case YandexPanel.PanelAction.StepForward:
            preview.step(1);
            break;
        case YandexPanel.PanelAction.Pick:
            preview.pickRequested(preview.item);
            break;
        }
        e.accepted = true;
    }
    // Nothing reaches the list below.
    MouseArea {
        anchors.fill: parent
        onWheel: wheel => wheel.accepted = true
    }

    RowLayout {
        id: previewBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 12
        spacing: 10

        Label {
            textFormat: Text.PlainText
            text: preview.item < 0 ? "" : preview.item < preview.otherCount ? words.phrase("ui-other-size", 0, ("" + (preview.item + 1)), ("" + (preview.otherCount))) : words.phrase("ui-similar-image", 0, ("" + (preview.item - preview.otherCount + 1)), ("" + (preview.similarCount)))
            font.bold: true
        }
        // The file's own size, upright, as the search decoded it (the tiles' size); Qt's of the loaded picture in the
        // moment before the search has it. Yandex's stated size is never shown.
        Label {
            textFormat: Text.PlainText
            text: preview.sizeText !== "" ? preview.sizeText : full.status === Image.Ready ? full.sourceSize.width + "×" + full.sourceSize.height : ""
            color: Theme.bodyText
        }
        Label {
            textFormat: Text.PlainText
            text: full.status === Image.Loading ? words.phrase("ui-loading-percent", 0, ("" + Math.round(full.progress * 100)), "") : full.status === Image.Error ? words.text("ui-cannot-decode") : ""
            color: full.status === Image.Error ? Theme.errorText : Theme.captionText
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        // No focus for the buttons: the arrow keys and Space stay the preview's.
        Button {
            text: preview.picked ? words.text("ui-picked") : words.text("ui-pick")
            highlighted: preview.picked
            focusPolicy: Qt.NoFocus
            onClicked: preview.pickRequested(preview.item)
        }
        Button {
            text: preview.actual ? words.text("ui-fit") : "1:1"
            enabled: full.zoomable
            focusPolicy: Qt.NoFocus
            onClicked: preview.actual = !preview.actual
        }
        Button {
            text: words.text("ui-back")
            focusPolicy: Qt.NoFocus
            onClicked: preview.close()
        }
    }

    Flickable {
        id: previewView
        anchors.fill: parent
        anchors.topMargin: previewBar.height + 24
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.bottomMargin: 12
        clip: true
        contentWidth: Math.max(width, full.width)
        contentHeight: Math.max(height, full.height)
        ScrollBar.vertical: ScrollBar {}
        ScrollBar.horizontal: ScrollBar {}

        // The wheel steps through the results and never scrolls the picture. Declared in the Flickable, its handler takes
        // the wheel before the Flickable scrolls (blocking by default); the content is at least the view, so it covers the
        // whole picture area in fit mode too.
        WheelStepper {
            id: wheelSteps
            onStepped: steps => preview.step(steps)
        }

        Image {
            id: full
            // Its size in logical pixels at 1:1, and the scale that fits it to the view, up or down.
            readonly property real naturalWidth: sourceSize.width / preview.dpr
            readonly property real naturalHeight: sourceSize.height / preview.dpr
            readonly property real fit: naturalWidth > 0 ? Math.min(previewView.width / naturalWidth, previewView.height / naturalHeight) : 1
            // 1:1 differs from the fit: it is worth a button and a click.
            readonly property bool zoomable: Math.abs(fit - 1) > 0.01
            readonly property real zoom: preview.actual ? 1 : fit

            source: preview.imageUrl
            asynchronous: true
            // Across a step the previous picture stays until the next one is decoded (no black flash while it loads).
            retainWhileLoading: true
            smooth: true
            mipmap: true
            width: naturalWidth * zoom
            height: naturalHeight * zoom
            x: Math.max(0, (previewView.width - width) / 2)
            y: Math.max(0, (previewView.height - height) / 2)

            TapHandler {
                onTapped: if (full.zoomable)
                    preview.actual = !preview.actual
            }
        }
    }
    BusyIndicator {
        anchors.centerIn: previewView
        running: full.status === Image.Loading
        visible: running
    }
    // The neighbours, decoded while hidden: the same URL and cache entry as `full` (no sourceSize), so a step finds it ready.
    Image {
        visible: false
        asynchronous: true
        cache: true
        source: preview.prevUrl
    }
    Image {
        visible: false
        asynchronous: true
        cache: true
        source: preview.nextUrl
    }
}
