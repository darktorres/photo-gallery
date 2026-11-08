// Reverse image search on Yandex for the photo in the viewer: its other sizes, and the images that look like it.
// Pick any of them (a click, Shift-click for a range, or a group's Select all), look at one at full size
// (double-click), and save the picks into a folder. The search, the results, the picks and the saving are the
// YandexSearch object's (src/yandex.rs), which works on background threads and announces each change: its rows are
// the results, so each ResultTile reads its roles, and the preview shows the item this file sets it to.
// Over a dimmed window, like the library panel. The results are ResultTile, the full-size view YandexPreview.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

Rectangle {
    Words {
        id: words
    }
    id: panel

    signal closed()

    // The gallery's row height and gap: the results are laid out with them, so they match the grid.
    required property real rowHeight
    required property real gap

    // The viewer's photo: the catalog's file, by its id.
    function show(fileId: int) {
        openPanel()
        yandex.searchFile(fileId)
    }

    // The command line's photo (`--find-similar`).
    function showCommandLine() {
        openPanel()
        yandex.searchCommandLine()
    }

    function openPanel() {
        preview.visible = false
        visible = true
        // The keys come here (Escape closes), not to the viewer's photo stepping behind.
        forceActiveFocus()
    }

    function close() {
        visible = false
        closed()
    }

    // For the test hook --yandex-preview: show result `item` at full size once the search has its results.
    property int previewWhenFound: -1
    Connections {
        target: yandex
        function onBusyChanged() {
            if (!yandex.busy && panel.previewWhenFound >= 0) {
                preview.open(panel.previewWhenFound)
                panel.previewWhenFound = -1
            }
        }
    }

    // A group's title with its count, and a button that picks all of it, or none once all are picked.
    component GroupHeader: RowLayout {
        id: header
        property string title
        property int first: 0
        property int count: 0
        // The group's picked results, which the search counts on each change.
        property int picked: 0

        visible: count > 0
        spacing: 12
        Label { text: header.title + " (" + header.count + ")"; font.bold: true }
        Button {
            text: header.picked === header.count ? words.text("ui-select-none") : words.text("ui-select-all")
            flat: true
            onClicked: yandex.setPicked(header.first, header.count, header.picked !== header.count)
        }
    }

    anchors.fill: parent
    visible: false
    color: Theme.panelScrim
    focus: visible
    // The panel's actions, as the codes keymap.panelAction returns (the Rust side lists them in the same order).
    enum PanelAction { Close, StepBack, StepForward, Pick }

    // The panel's keys: the key map (pg-client's keys) names the action.
    KeyMap {
        id: keymap
    }

    Keys.onPressed: (e) => {
        if (keymap.panelAction(e.key) !== YandexPanel.PanelAction.Close)
            return
        close()
        e.accepted = true
    }
    // Clicks on the dimmed background close it. No click or wheel reaches the viewer below: the results' Flickable
    // passes the wheel on at its ends, and the viewer would step to the next picture (its wheel), so it is not reached.
    MouseArea {
        anchors.fill: parent
        onClicked: panel.close()
        onWheel: (wheel) => wheel.accepted = true
    }

    // The results are its rows, and its state comes across when it changes (the change pump, src/yandex.rs). They are
    // laid out in the width of the list they are in, at the gallery's row height and gap (their frames).
    YandexSearch {
        id: yandex
        layoutWidth: sections.width
        rowHeight: panel.rowHeight
        gap: panel.gap
    }

    // Where the picked results are saved: the picker asks for the folder, then they are saved into it (`done`).
    FolderPicker {
        id: savePicker
        onDone: yandex.downloadPicked()
        onOpenInApp: saveDialog.open = true
        // The save's folder dialog couldn't open: the status line says why (this picker is only the save's).
        onFailed: (purpose, message) => yandex.showFailure(message)
    }
    FolderPickerDialog {
        id: saveDialog
        z: 100
        names: savePicker.names
        here: savePicker.here
        onGoUp: savePicker.up()
        onOpenRow: (row) => savePicker.enter(row)
        onChooseRow: (row) => savePicker.choose(row)
        onChooseHere: savePicker.chooseHere()
        onCancelled: savePicker.cancel()
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 64, 1040)
        height: Math.min(parent.height - 64, 760)
        padding: 16
        // Clicks inside the panel don't reach the dimmed background.
        MouseArea { anchors.fill: parent }

        ColumnLayout {
            anchors.fill: parent
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: 10
                Label { text: words.text("ui-find-similar-yandex"); font.bold: true; font.pixelSize: 16 }
                BusyIndicator {
                    running: yandex.busy
                    visible: running
                    implicitWidth: 28
                    implicitHeight: 28
                }
                Item { Layout.fillWidth: true }
                Button {
                    text: words.phrase("ui-save-picked", yandex.pickedCount, ("" + (yandex.pickedCount)), "")
                    enabled: yandex.pickedCount > 0 && !yandex.busy
                    onClicked: {
                        // The picker opens in the folder saved into last, or the photo's.
                        yandex.prepareSave()
                        savePicker.pick(2)
                    }
                }
                Button { text: words.text("ui-close"); onClicked: panel.close() }
            }

            // How to see a result at full size, shown while there are results to see.
            Label {
                Layout.fillWidth: true
                visible: yandex.otherCount + yandex.similarCount > 0
                text: words.text("ui-yandex-hint")
                color: Theme.hintText
                font.pixelSize: 13
            }

            // The status in full: wrapped, so nothing is cut off, and selectable, so it can be copied.
            TextEdit {
                Layout.fillWidth: true
                visible: yandex.status !== ""
                text: yandex.status
                color: Theme.bodyText
                font.pixelSize: 13
                wrapMode: TextEdit.Wrap
                readOnly: true
                selectByMouse: true
            }

            Flickable {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                contentWidth: width
                contentHeight: sections.height
                ScrollBar.vertical: ScrollBar {}

                ColumnLayout {
                    id: sections
                    width: parent.width
                    spacing: 12

                    GroupHeader {
                        title: words.text("ui-other-sizes")
                        count: yandex.otherCount
                        picked: yandex.otherPicked
                    }
                    // The group's block: its results at their frames, which YandexSearch lays out in the panel's width.
                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: yandex.otherHeight
                        Repeater {
                            model: yandex
                            ResultTile {
                                visible: index < yandex.otherCount
                                onToggled: (chosen) => yandex.togglePick(chosen)
                                onRangeTo: (chosen) => yandex.pickTo(chosen)
                                onPreviewRequested: (chosen) => preview.open(chosen)
                            }
                        }
                    }

                    GroupHeader {
                        title: words.text("ui-similar-images")
                        first: yandex.otherCount
                        count: yandex.similarCount
                        picked: yandex.similarPicked
                    }
                    Item {
                        Layout.fillWidth: true
                        Layout.preferredHeight: yandex.similarHeight
                        Repeater {
                            model: yandex
                            ResultTile {
                                visible: index >= yandex.otherCount
                                onToggled: (chosen) => yandex.togglePick(chosen)
                                onRangeTo: (chosen) => yandex.pickTo(chosen)
                                onPreviewRequested: (chosen) => preview.open(chosen)
                            }
                        }
                    }

                    Label {
                        text: words.text("ui-no-results")
                        color: Theme.hintText
                        visible: !yandex.busy && yandex.otherCount === 0 && yandex.similarCount === 0
                            && yandex.status === ""
                    }
                }
            }
        }
    }

    // One result at full size: see YandexPreview.
    YandexPreview {
        id: preview
        otherCount: yandex.otherCount
        similarCount: yandex.similarCount
        imageUrl: yandex.previewUrl
        sizeText: yandex.previewWidth > 0 ? yandex.previewWidth + "×" + yandex.previewHeight : ""
        picked: yandex.previewPicked
        onPickRequested: (chosen) => yandex.togglePick(chosen)
        onClosed: panel.forceActiveFocus()
    }
    // The search previews the item the preview is open on, and none while it is closed.
    Binding {
        target: yandex
        property: "previewItem"
        value: preview.visible ? preview.item : -1
    }
}
