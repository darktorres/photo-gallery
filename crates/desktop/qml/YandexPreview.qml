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

    // The panel's actions, as the codes keymap.panelAction returns (the Rust side lists them in the same order).
    enum PanelAction { Close, StepBack, StepForward, Pick }

    // The result shown (its index in the search), -1 when none.
    property int item: -1
    property bool actual: false

    // The results: the other sizes first, then the similar images.
    required property int otherCount
    required property int similarCount
    // The item shown: its picture's URL (`""` while hidden), its size as text (`""` when unknown), and whether it is picked.
    required property string imageUrl
    required property string sizeText
    required property bool picked

    // The user asked to pick or unpick an item (`-1` for none).
    signal pickRequested(int item)
    // Back to the list.
    signal closed()

    readonly property int total: preview.otherCount + preview.similarCount

    // The keys: the key map (pg-client's keys) names the action.
    KeyMap {
        id: keymap
    }

    function open(i: int) {
        preview.item = i
        preview.actual = false
        preview.visible = true
        preview.forceActiveFocus()
    }

    function close() {
        preview.visible = false
        preview.item = -1
        preview.closed()
    }

    function step(by: int) {
        if (preview.total > 0) {
            preview.item = (preview.item + by + preview.total) % preview.total
            preview.actual = false
        }
    }

    // 1:1 opens on the image's middle; fitted, the content is the view, scrolled to its start. Done once the
    // content has its new size, on the next event-loop pass.
    Timer {
        id: centre
        interval: 0
        onTriggered: {
            previewView.contentX = Math.max(0, (previewView.contentWidth - previewView.width) / 2)
            previewView.contentY = Math.max(0, (previewView.contentHeight - previewView.height) / 2)
        }
    }
    onActualChanged: centre.restart()

    anchors.fill: parent
    visible: false
    color: Theme.previewFill
    Keys.onPressed: (e) => {
        const action = keymap.panelAction(e.key)
        if (action < 0)
            return
        switch (action) {
        case YandexPreview.PanelAction.Close: preview.close(); break
        case YandexPreview.PanelAction.StepBack: preview.step(-1); break
        case YandexPreview.PanelAction.StepForward: preview.step(1); break
        case YandexPreview.PanelAction.Pick: preview.pickRequested(preview.item); break
        }
        e.accepted = true
    }
    // Nothing reaches the list below.
    MouseArea {
        anchors.fill: parent
        onWheel: (wheel) => wheel.accepted = true
    }

    RowLayout {
        id: previewBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 12
        spacing: 10

        Label {
            text: preview.item < 0 ? ""
                : preview.item < preview.otherCount
                    ? "Other size " + (preview.item + 1) + " of " + preview.otherCount
                    : "Similar image " + (preview.item - preview.otherCount + 1) + " of " + preview.similarCount
            font.bold: true
        }
        // The file's own size once it is loaded; Yandex's until then.
        Label {
            text: full.status === Image.Ready ? full.sourceSize.width + "×" + full.sourceSize.height
                : preview.sizeText
            color: Theme.bodyText
        }
        Label {
            text: full.status === Image.Loading ? "Loading… " + Math.round(full.progress * 100) + "%"
                : full.status === Image.Error ? "Can't load or decode it; saving may still work." : ""
            color: full.status === Image.Error ? Theme.errorText : Theme.captionText
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        // No focus for the buttons: the arrow keys and Space stay the preview's.
        Button {
            text: preview.picked ? "Picked ✓" : "Pick"
            highlighted: preview.picked
            focusPolicy: Qt.NoFocus
            onClicked: preview.pickRequested(preview.item)
        }
        Button {
            text: preview.actual ? "Fit" : "1:1"
            enabled: full.zoomable
            focusPolicy: Qt.NoFocus
            onClicked: preview.actual = !preview.actual
        }
        Button { text: "Back"; focusPolicy: Qt.NoFocus; onClicked: preview.close() }
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

        Image {
            id: full
            // Its size in logical pixels at 1:1, and the scale that fits it to the view, up or down.
            readonly property real naturalWidth: sourceSize.width / Screen.devicePixelRatio
            readonly property real naturalHeight: sourceSize.height / Screen.devicePixelRatio
            readonly property real fit: naturalWidth > 0
                ? Math.min(previewView.width / naturalWidth, previewView.height / naturalHeight) : 1
            // 1:1 differs from the fit: it is worth a button and a click.
            readonly property bool zoomable: Math.abs(fit - 1) > 0.01
            readonly property real zoom: preview.actual ? 1 : fit

            source: preview.imageUrl
            asynchronous: true
            smooth: true
            mipmap: true
            width: naturalWidth * zoom
            height: naturalHeight * zoom
            x: Math.max(0, (previewView.width - width) / 2)
            y: Math.max(0, (previewView.height - height) / 2)

            TapHandler {
                onTapped: if (full.zoomable) preview.actual = !preview.actual
            }
        }
    }
    BusyIndicator {
        anchors.centerIn: previewView
        running: full.status === Image.Loading
        visible: running
    }
}
