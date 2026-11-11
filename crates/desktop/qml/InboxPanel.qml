// The inbox, over a dimmed window: what the app told the user, newest first (Notices, src/notices.rs). A row is the
// notice's title, where it comes from, when it was last posted, and how many times; the selected row's details open
// below it, with Copy and Dismiss, which the panel asks of Main (its signals), as Clear all. A ledger is one row: its
// details are what it lists (FailedFiles, src/failed_files.rs: a pass's failed files, or the folders the scans couldn't
// take, under their library folders), "loading" until the list's read is delivered, each with Reveal; the row has Retry
// (the thumbnails) or "Show the failed files" (the import). Opening it marks what it shows read (Main: Notices'
// setViewing). Made the first time it is opened (Main's inboxLoader), then kept. Escape or Close dismisses it; Tab stays
// within it.
//
// The selection is an entry (its `key` role) at its row, which the panel follows as rows are inserted, moved and removed:
// the model moves rows rather than resetting them (list_diff), so a new notice keeps the selection and the details open. The keys, on the list: Up, Down, Home, End,
// Page Up and Page Down move; Enter or Space open and close the details; Delete dismisses; Ctrl+C copies; R retries.
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
    // The entries (Main's Notices), and what the ledger's entry shown lists (Main's FailedFiles), of which the ledger
    // holds `failedTotal`; whether that list is being read, and the error its read failed with ("": it didn't).
    required property var notices
    required property var failedFiles
    required property int failedTotal
    required property bool failedLoading
    required property string failedError
    // The selected row (-1: none), and what it says: its key, its details, and for a ledger its pass (-1: not one), its
    // newest failure and what it offers.
    property int selected: -1
    property string selectedKey: ""
    property string selectedText: ""
    property int selectedPass: -1
    property real selectedSerial: 0
    property bool selectedCanRetry: false
    property bool selectedCanShowFiles: false
    // The selected row's details are open.
    property bool detailsOpen: false
    // The row to open when the panel opens, or when it appears (the status bar's "N failed": Main finds the pass's
    // ledger; -1: none).
    property int expandRow: -1
    // The ledger's list last asked for (`filesRequested`): its entry's key ("" for none) and its newest failure then.
    property string requestedKey: ""
    property real requestedSerial: -1

    // The entry at `row` is to be copied, dismissed or retried; every entry shown is to be dismissed.
    signal copyRequested(int row)
    signal dismissRequested(int row)
    signal retryRequested(int row)
    signal clearRequested()
    // A ledger's entry is open: its list is to be read (-1: none). Asked again when its newest failure moves.
    signal filesRequested(int pass)
    // A failed file is to be shown in its folder; a failed folder's parent (its row of the list); the import's failed
    // files are to be listed in the grid.
    signal revealRequested(real fileId)
    signal folderRevealRequested(int row)
    signal showFilesRequested()

    // The selected row says what it is (when it becomes the selected one, and when its entry changes in place).
    function show(key: string, body: string, pass: int, serial: real, canRetry: bool, canShowFiles: bool) {
        selectedKey = key
        selectedText = body
        selectedPass = pass
        selectedSerial = serial
        selectedCanRetry = canRetry
        selectedCanShowFiles = canShowFiles
        requestFiles()
    }

    // The open ledger's list, asked for once per entry and newest failure; none while no ledger's details are open.
    function requestFiles() {
        const pass = detailsOpen && selected >= 0 ? selectedPass : -1
        const key = pass >= 0 ? selectedKey : ""
        if (key === requestedKey && (key === "" || selectedSerial === requestedSerial))
            return
        requestedKey = key
        requestedSerial = selectedSerial
        filesRequested(pass)
    }
    onDetailsOpenChanged: requestFiles()

    function unselect() {
        selected = -1
        detailsOpen = false
        selectedKey = ""
        selectedText = ""
        selectedPass = -1
        selectedCanRetry = false
        selectedCanShowFiles = false
        requestFiles()
    }

    // The row just selected is to tell what it is (its delegate answers; one made later tells on its own).
    signal rowSelected(int row)

    // Select `row`.
    function select(row: int) {
        selected = row
        selectedKey = ""
        rowSelected(row)
    }

    // Select `row` and bring it into view (a row past the list selects the nearest). Positioning first makes the row's
    // delegate, at its row (the view applies the model's changes then).
    function moveTo(row: int) {
        if (list.count === 0)
            return
        const at = Math.max(0, Math.min(list.count - 1, row))
        list.positionViewAtIndex(at, ListView.Contain)
        select(at)
    }

    function dismissSelected() {
        if (selected < 0)
            return
        const row = selected
        unselect()
        dismissRequested(row)
    }

    function retrySelected() {
        if (selected >= 0 && selectedCanRetry)
            retryRequested(selected)
    }

    // The entry to expand was opened: once per opening of the panel and per appearance of the entry, so its row moving
    // later (a notice coming in above it) doesn't take the user's selection back to it.
    property bool expanded: false
    function expand() {
        if (!open || expandRow < 0) {
            expanded = false
            return
        }
        if (expanded || expandRow >= list.count)
            return
        moveTo(expandRow)
        detailsOpen = true
        expanded = true
    }
    onOpenChanged: expand()
    onExpandRowChanged: expand()
    Component.onCompleted: expand()

    // The rows move under the selection, which follows its entry's row as they do. Not the view's current row: a ListView
    // applies the model's changes at its next polish (and hears of them after these handlers), its current row and its
    // delegates' rows stale until then, so a key or a click in between would act on another entry. The selected row
    // dismissed lets the selection go; a reset (Clear all) takes every row.
    Connections {
        target: panel.notices
        function onRowsInserted(parent: var, first: int, last: int) {
            if (panel.selected >= first)
                panel.selected += last - first + 1
        }
        function onRowsRemoved(parent: var, first: int, last: int) {
            if (panel.selected < 0)
                return
            if (panel.selected > last)
                panel.selected -= last - first + 1
            else if (panel.selected >= first)
                panel.unselect()
        }
        // Rows `start` to `end` go before row `row` (counted before the move).
        function onRowsMoved(parent: var, start: int, end: int, destination: var, row: int) {
            const at = panel.selected
            const n = end - start + 1
            if (at < 0)
                return
            if (at >= start && at <= end)
                panel.selected = (row > end ? row - n : row) + at - start
            else if (row > end && at > end && at < row)
                panel.selected = at - n
            else if (row < start && at >= row && at < start)
                panel.selected = at + n
        }
        function onModelReset() {
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
                // The selection is the panel's (`selected`), not the view's current row.
                currentIndex: -1
                // The keys go to the list when the panel opens (ModalOverlay is a focus scope).
                focus: true
                activeFocusOnTab: true
                KeyNavigation.backtab: closeButton
                ScrollBar.vertical: ScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: words.text("ui-inbox-title")
                readonly property int rowHeight: 30
                Keys.onPressed: (event) => {
                    const page = Math.max(1, Math.floor(list.height / list.rowHeight) - 1)
                    const at = panel.selected
                    const ctrl = (event.modifiers & Qt.ControlModifier) !== 0
                    if (event.key === Qt.Key_Up)
                        panel.moveTo(at - 1)
                    else if (event.key === Qt.Key_Down)
                        panel.moveTo(at + 1)
                    else if (event.key === Qt.Key_Home)
                        panel.moveTo(0)
                    else if (event.key === Qt.Key_End)
                        panel.moveTo(list.count - 1)
                    else if (event.key === Qt.Key_PageUp)
                        panel.moveTo(at - page)
                    else if (event.key === Qt.Key_PageDown)
                        panel.moveTo(Math.max(at, 0) + page)
                    else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)
                        panel.detailsOpen = at >= 0 && !panel.detailsOpen
                    else if (event.key === Qt.Key_Delete)
                        panel.dismissSelected()
                    else if (event.key === Qt.Key_C && ctrl) {
                        if (at >= 0)
                            panel.copyRequested(at)
                    } else if (event.key === Qt.Key_R && event.modifiers === Qt.NoModifier)
                        panel.retrySelected()
                    else
                        return
                    event.accepted = true
                }
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
                    required property string key
                    required property string sinceDismissed
                    required property real serial
                    // The selected entry's row: by its key, which a stale row number can't confuse.
                    readonly property bool chosen: panel.selectedKey.length > 0 && row.key === panel.selectedKey
                    function tell() {
                        panel.show(row.key, row.body, row.pass, row.serial, row.canRetry, row.canShowFiles)
                    }
                    // The row just selected, whose key isn't known yet: it tells it.
                    function wanted(): bool {
                        return panel.selectedKey.length === 0 && row.index === panel.selected
                    }
                    Connections {
                        target: panel
                        function onRowSelected(at: int) {
                            if (at === row.index)
                                row.tell()
                        }
                    }
                    // The entry changed in place (a new revision, its counts, a newer failure): its details follow.
                    onBodyChanged: if (chosen) tell()
                    onSerialChanged: if (chosen) tell()
                    onCanRetryChanged: if (chosen) tell()
                    Component.onCompleted: if (chosen || wanted()) tell()
                    width: list.width
                    height: list.rowHeight
                    color: row.chosen ? "#2f5f93" : (row.index % 2 ? "#242424" : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: row.title
                    Accessible.description: row.source
                    Accessible.selected: row.chosen
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
                        // A ledger back after a dismissal: how many came since.
                        Label {
                            visible: row.sinceDismissed.length > 0
                            text: row.sinceDismissed
                            textFormat: Text.PlainText
                            color: "#e0a050"
                        }
                        // Not counted (-1: a ledger's counts while the inbox isn't viewed) shows nothing.
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
                            list.forceActiveFocus()
                            panel.select(row.index)
                            panel.detailsOpen = true
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
            // The selected row's details, selectable; a ledger's, what it lists.
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 110
                visible: panel.detailsOpen && panel.selected >= 0 && panel.selectedPass < 0
                TextArea {
                    objectName: "inboxDetails"
                    text: panel.selectedText
                    textFormat: TextEdit.PlainText
                    readOnly: true
                    wrapMode: TextEdit.Wrap
                    activeFocusOnTab: false
                }
            }
            ColumnLayout {
                objectName: "inboxLedger"
                visible: panel.detailsOpen && panel.selected >= 0 && panel.selectedPass >= 0
                Layout.fillWidth: true
                spacing: 4
                Label {
                    text: panel.selectedText
                    textFormat: Text.PlainText
                    color: "#aaaaaa"
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }
                // Until the list's read is delivered (off the window's thread).
                Label {
                    objectName: "inboxLoading"
                    visible: panel.failedLoading
                    text: words.text("ui-inbox-loading")
                    textFormat: Text.PlainText
                    color: "#888888"
                }
                Label {
                    objectName: "inboxFilesError"
                    visible: panel.failedError.length > 0
                    text: words.phrase("ui-inbox-unreadable", 0, panel.failedError, "")
                    textFormat: Text.PlainText
                    color: "#e06060"
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
                    // The scans' folders, under the library folder that holds each (a file has none).
                    section.property: "root"
                    section.delegate: Label {
                        required property string section
                        visible: section.length > 0
                        height: visible ? implicitHeight + 4 : 0
                        text: section
                        textFormat: Text.PlainText
                        font.bold: true
                        color: "#aaaaaa"
                    }
                    delegate: Item {
                        id: file
                        required property int index
                        required property string path
                        required property string error
                        required property real fileId
                        required property bool isNew
                        required property bool isFolder
                        required property string reason
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
                                    text: file.isFolder ? file.reason + ": " + file.error : file.error
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
                                onClicked: {
                                    if (file.isFolder)
                                        panel.folderRevealRequested(file.index)
                                    else
                                        panel.revealRequested(file.fileId)
                                }
                            }
                        }
                    }
                }
                Label {
                    visible: panel.failedTotal > files.count && !panel.failedLoading
                    // The scans' folders (pass 3), or a pass's files.
                    text: panel.selectedPass === 3
                        ? words.phrase("ui-inbox-more-folders", panel.failedTotal, "" + panel.failedTotal, "" + files.count)
                        : words.phrase("ui-inbox-more", panel.failedTotal, "" + panel.failedTotal, "" + files.count)
                    textFormat: Text.PlainText
                    color: "#888888"
                }
            }
            RowLayout {
                Button {
                    objectName: "inboxRetry"
                    visible: panel.selectedCanRetry
                    text: words.text("ui-inbox-retry")
                    onClicked: panel.retrySelected()
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
                    onClicked: panel.dismissSelected()
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
