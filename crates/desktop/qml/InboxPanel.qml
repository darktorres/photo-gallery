// The inbox, over a dimmed window: what the app told the user, newest first (Notices, src/notices.rs). A row is the
// notice's title, where it comes from, when it was last posted, and how many times; a click shows its details below,
// with Copy and Dismiss, which the panel asks of Main (its signals), as Clear all. A pass's failure ledger is one row: its
// details are its failed files (FailedFiles, src/failed_files.rs), each with Reveal, and the row has Retry (the
// thumbnails) or "Show the failed files" (the import). Opening it marks every entry read. Made the first time it is
// opened (Main's inboxLoader), then kept. Escape or Close dismisses it; Tab stays within it.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

ModalOverlay {
    Words {
        id: words
    }
    id: panel
    // The entries (Main's Notices), and the failed files of the ledger's entry shown (Main's FailedFiles), of which the
    // ledger holds `failedTotal`.
    required property var notices
    required property var failedFiles
    required property int failedTotal
    // The row clicked: its details are shown below the list (-1: none). A ledger's row is its pass (-1: not one), with
    // what it offers.
    property int selected: -1
    property string selectedText: ""
    property int selectedPass: -1
    property bool selectedCanRetry: false
    property bool selectedCanShowFiles: false
    // The row to open when the panel opens (the status bar's "N failed"; -1: none).
    property int expandRow: -1

    // The entry at `row` is to be copied, dismissed or retried; every entry is to be dismissed.
    signal copyRequested(int row)
    signal dismissRequested(int row)
    signal retryRequested(int row)
    signal clearRequested()
    // A ledger's entry is shown: its pass's files are to be listed (-1: none).
    signal filesRequested(int pass)
    // A failed file is to be shown in its folder; the import's failed files are to be listed in the grid.
    signal revealRequested(real fileId)
    signal showFilesRequested()

    // The row at `row` (selected) says what it is: its details, and for a ledger its files.
    function show(row: int, body: string, pass: int, canRetry: bool, canShowFiles: bool) {
        selected = row
        selectedText = body
        selectedCanRetry = canRetry
        selectedCanShowFiles = canShowFiles
        selectedPass = pass
        filesRequested(pass)
    }

    function unselect() {
        selected = -1
        selectedPass = -1
        selectedCanRetry = false
        selectedCanShowFiles = false
        filesRequested(-1)
    }

    function expand() {
        if (!open || expandRow < 0)
            return
        selected = expandRow
        list.positionViewAtIndex(expandRow, ListView.Contain)
    }
    onOpenChanged: expand()
    onExpandRowChanged: expand()
    Component.onCompleted: expand()

    // A new listing moves the rows: the details shown are of a row that may be another now. A ledger's row keeps its
    // place by its pass (its delegate finds it, below); any other is let go.
    Connections {
        target: panel.notices
        function onModelReset() {
            if (panel.selectedPass < 0)
                panel.unselect()
        }
    }

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1000)
        height: Math.min(parent.height - 80, 700)
        padding: 16
        // A control is a focus scope too: the focus passes through it to the list.
        focus: true
        MouseArea { anchors.fill: parent }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { text: words.text("ui-inbox-title"); font.bold: true; font.pixelSize: 16 }
            WheelListView {
                id: list
                objectName: "inboxList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: panel.notices
                // The keys go to the list when the panel opens (ModalOverlay is a focus scope).
                focus: true
                activeFocusOnTab: true
                KeyNavigation.backtab: closeButton
                ScrollBar.vertical: ScrollBar {}
                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string title
                    required property string body
                    required property string source
                    required property int level
                    required property real updated
                    required property int count
                    required property bool read
                    required property int pass
                    required property bool canRetry
                    required property bool canShowFiles
                    readonly property bool chosen: panel.selected === row.index
                    function tell() { panel.show(row.index, row.body, row.pass, row.canRetry, row.canShowFiles) }
                    onChosenChanged: if (chosen) tell()
                    Component.onCompleted: {
                        // The ledger's entry shown, after a new listing: wherever it is now.
                        if (row.pass >= 0 && row.pass === panel.selectedPass && !chosen)
                            panel.selected = row.index
                        else if (chosen)
                            tell()
                    }
                    width: list.width
                    height: 30
                    color: panel.selected === row.index ? "#2f5f93" : (row.index % 2 ? "#242424" : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.title
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 10
                        Rectangle {
                            Layout.preferredWidth: 8
                            Layout.preferredHeight: 8
                            radius: 4
                            color: row.level === 2 ? "#e06060" : (row.level === 1 ? "#e0a050" : "#6a9fd8")
                        }
                        Label {
                            text: row.title
                            textFormat: Text.PlainText
                            font.bold: !row.read
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        Label {
                            visible: row.count > 1
                            text: "×" + row.count
                            textFormat: Text.PlainText
                            color: "#aaaaaa"
                        }
                        Label {
                            text: row.source
                            textFormat: Text.PlainText
                            color: "#aaaaaa"
                            Layout.preferredWidth: 120
                            elide: Text.ElideRight
                        }
                        Label {
                            text: Qt.formatDateTime(new Date(row.updated * 1000), "yyyy-MM-dd hh:mm")
                            textFormat: Text.PlainText
                            color: "#888888"
                        }
                    }
                    TapHandler {
                        onTapped: {
                            if (row.chosen)
                                row.tell()
                            else
                                panel.selected = row.index
                        }
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: words.text("ui-inbox-empty")
                    color: "#888888"
                }
            }
            // The row clicked: its details, selectable; a ledger's, its files.
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 110
                visible: panel.selected >= 0 && panel.selectedPass < 0
                TextArea {
                    text: panel.selectedText
                    textFormat: TextEdit.PlainText
                    readOnly: true
                    wrapMode: TextEdit.Wrap
                    activeFocusOnTab: false
                }
            }
            ColumnLayout {
                visible: panel.selected >= 0 && panel.selectedPass >= 0
                Layout.fillWidth: true
                spacing: 4
                Label {
                    text: panel.selectedText
                    textFormat: Text.PlainText
                    color: "#aaaaaa"
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }
                WheelListView {
                    id: files
                    objectName: "inboxFiles"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 180
                    clip: true
                    model: panel.failedFiles
                    activeFocusOnTab: false
                    ScrollBar.vertical: ScrollBar {}
                    delegate: Item {
                        id: file
                        required property int index
                        required property string path
                        required property string error
                        required property real fileId
                        required property bool isNew
                        width: files.width
                        height: 40
                        Accessible.role: Accessible.ListItem
                        Accessible.name: file.path
                        RowLayout {
                            anchors.fill: parent
                            anchors.rightMargin: 8
                            spacing: 8
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Label {
                                    text: file.path
                                    textFormat: Text.PlainText
                                    font.bold: file.isNew
                                    elide: Text.ElideMiddle
                                    Layout.fillWidth: true
                                }
                                Label {
                                    text: file.error
                                    textFormat: Text.PlainText
                                    color: "#aaaaaa"
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                            }
                            Button {
                                objectName: "inboxReveal" + file.index
                                text: words.text("ui-inbox-reveal")
                                flat: true
                                onClicked: panel.revealRequested(file.fileId)
                            }
                        }
                    }
                }
                Label {
                    visible: panel.failedTotal > files.count
                    text: words.phrase("ui-inbox-more", panel.failedTotal, "" + panel.failedTotal, "" + files.count)
                    textFormat: Text.PlainText
                    color: "#888888"
                }
            }
            RowLayout {
                Button {
                    objectName: "inboxRetry"
                    visible: panel.selectedCanRetry
                    text: words.text("ui-inbox-retry")
                    onClicked: {
                        const row = panel.selected
                        panel.unselect()
                        panel.retryRequested(row)
                    }
                }
                Button {
                    objectName: "inboxShowFiles"
                    visible: panel.selectedCanShowFiles
                    text: words.text("ui-inbox-show-files")
                    onClicked: panel.showFilesRequested()
                }
                Button {
                    objectName: "inboxCopy"
                    text: words.text("ui-inbox-copy")
                    enabled: panel.selected >= 0
                    onClicked: panel.copyRequested(panel.selected)
                }
                Button {
                    objectName: "inboxDismiss"
                    text: words.text("ui-inbox-dismiss")
                    enabled: panel.selected >= 0
                    onClicked: {
                        const row = panel.selected
                        panel.unselect()
                        panel.dismissRequested(row)
                    }
                }
                Item { Layout.fillWidth: true }
                Button {
                    objectName: "inboxClear"
                    text: words.text("ui-inbox-clear")
                    enabled: list.count > 0
                    onClicked: {
                        panel.unselect()
                        panel.clearRequested()
                    }
                }
                Button {
                    id: closeButton
                    objectName: "inboxClose"
                    text: words.text("ui-close")
                    activeFocusOnTab: true
                    KeyNavigation.tab: list
                    onClicked: panel.dismissed()
                }
            }
        }
    }
}
