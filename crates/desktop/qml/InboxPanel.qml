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
    id: panel
    Words {
        id: words
    }
    // The entries (Main's Notices), and what the ledger's entry shown lists (Main's FailedFiles), of which the ledger
    // holds `failedTotal`; whether that list is being read, and the error its read failed with ("": it didn't).
    required property var notices
    required property var failedFiles
    required property int failedTotal
    required property bool failedLoading
    required property string failedError
    // The selection is the notice model's, and the window gives the panel what it shows: the selected row, its key, its
    // text, its pass, its newest failure, what it offers, and whether its details are open. The panel asks for each change
    // (the signals below): the model holds the selection, so it follows its entry as the entries move.
    required property int selected
    required property string selectedKey
    required property string selectedText
    required property int selectedPass
    required property real selectedSerial
    required property bool selectedCanRetry
    required property bool selectedCanShowFiles
    required property bool detailsOpen
    // The row to open when the panel opens, or when it appears (the status bar's "N failed": Main finds the pass's
    // ledger; -1: none).
    property int expandRow: -1
    // The ledger code of the scans' folders (`ledgers::kind_code` on Rust's `LedgerKind::ScanFolders`, which the ledgers'
    // test pins to this value): its entry lists folders, not files.
    readonly property int scanFoldersCode: 3

    // The entry at `row` is to be copied, dismissed or retried; every entry shown is to be dismissed.
    signal copyRequested(int row)
    signal dismissRequested(int row)
    signal retryRequested(int row)
    signal clearRequested
    // A failed file is to be shown in its folder; a failed folder's parent (its row of the list); the import's failed
    // files are to be listed in the grid.
    signal revealRequested(real fileId)
    signal folderRevealRequested(int row)
    signal showFilesRequested
    // The selection and the details are the notices' (Main's Notices), so the panel asks the window for each change.
    signal selectRequested(int row)
    signal unselectRequested
    signal detailsOpenRequested(bool open)
    signal dismissSelectedRequested
    signal retrySelectedRequested

    // Select `row` (none for a row past the list). The notice model holds the selection, and its details.
    function select(row: int) {
        panel.selectRequested(row);
    }

    // Select `row` and bring it into view (a row past the list selects the nearest). forceLayout applies the model's
    // changes and creates the row's delegate now, so the row is there to be selected.
    function moveTo(row: int) {
        if (list.count === 0)
            return;
        const at = Math.max(0, Math.min(list.count - 1, row));
        list.positionViewAtIndex(at, ListView.Contain);
        list.forceLayout();
        select(at);
    }

    // The window acts on the selected entry: nothing is asked for with no selection, and a retry only for an entry that
    // offers one (the button shows for the same case).
    function dismissSelected() {
        if (panel.selected >= 0)
            panel.dismissSelectedRequested();
    }

    function retrySelected() {
        if (panel.selected >= 0 && panel.selectedCanRetry)
            panel.retrySelectedRequested();
    }

    // The entry to expand was opened: once per opening of the panel and per appearance of the entry, so its row moving
    // later (a notice coming in above it) doesn't take the user's selection back to it.
    property bool expanded: false
    function expand() {
        if (!open || expandRow < 0) {
            expanded = false;
            return;
        }
        if (expanded || expandRow >= list.count)
            return;
        moveTo(expandRow);
        panel.detailsOpenRequested(true);
        expanded = true;
    }
    onOpenChanged: expand()
    onExpandRowChanged: expand()
    Component.onCompleted: expand()

    Pane {
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1000)
        height: Math.min(parent.height - 80, 700)
        padding: 16
        // A control is a focus scope too: the focus passes through it to the list.
        focus: true
        MouseArea {
            anchors.fill: parent
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label {
                text: words.text("ui-inbox-title")
                font.bold: true
                font.pixelSize: 16
            }
            WheelListView {
                id: list
                objectName: "inboxList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: panel.notices
                // An entry to expand that was past the list's end is expanded once the list has it.
                onCountChanged: panel.expand()
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
                Keys.onPressed: event => {
                    const page = Math.max(1, Math.floor(list.height / list.rowHeight) - 1);
                    const at = panel.selected;
                    const ctrl = (event.modifiers & Qt.ControlModifier) !== 0;
                    if (event.key === Qt.Key_Up)
                        panel.moveTo(at - 1);
                    else if (event.key === Qt.Key_Down)
                        panel.moveTo(at + 1);
                    else if (event.key === Qt.Key_Home)
                        panel.moveTo(0);
                    else if (event.key === Qt.Key_End)
                        panel.moveTo(list.count - 1);
                    else if (event.key === Qt.Key_PageUp)
                        panel.moveTo(at - page);
                    else if (event.key === Qt.Key_PageDown)
                        panel.moveTo(Math.max(at, 0) + page);
                    else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)
                        panel.detailsOpenRequested(at >= 0 && !panel.detailsOpen);
                    else if (event.key === Qt.Key_Delete)
                        panel.dismissSelected();
                    else if (event.key === Qt.Key_C && ctrl) {
                        if (at >= 0)
                            panel.copyRequested(at);
                    } else if (event.key === Qt.Key_R && event.modifiers === Qt.NoModifier)
                        panel.retrySelected();
                    else
                        return;
                    event.accepted = true;
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
                    width: list.width
                    height: list.rowHeight
                    color: row.chosen ? Theme.chosenRow : (row.index % 2 ? "#242424" : "transparent")
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
                            color: row.level === 2 ? Theme.invalidText : (row.level === 1 ? Theme.statusWarning : "#6a9fd8")
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
                            color: Theme.statusWarning
                        }
                        // Not counted (-1: a ledger's counts while the inbox isn't viewed) shows nothing.
                        Label {
                            visible: row.count > 1
                            text: "×" + words.count(row.count)
                            textFormat: Text.PlainText
                            color: Theme.statusText
                        }
                        Label {
                            text: row.source
                            textFormat: Text.PlainText
                            color: Theme.statusText
                            Layout.preferredWidth: 120
                            elide: Text.ElideRight
                        }
                        Label {
                            text: Qt.formatDateTime(new Date(row.updated * 1000), "yyyy-MM-dd hh:mm")
                            textFormat: Text.PlainText
                            color: Theme.hintText
                        }
                    }
                    TapHandler {
                        onTapped: {
                            list.forceActiveFocus();
                            panel.select(row.index);
                            panel.detailsOpenRequested(true);
                        }
                    }
                }
                Label {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: words.text("ui-inbox-empty")
                    color: Theme.hintText
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
                    color: Theme.statusText
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }
                // Until the list's read is delivered (off the window's thread).
                Label {
                    objectName: "inboxLoading"
                    visible: panel.failedLoading
                    text: words.text("ui-loading")
                    textFormat: Text.PlainText
                    color: Theme.hintText
                }
                Label {
                    objectName: "inboxFilesError"
                    visible: panel.failedError.length > 0
                    text: words.phrase("ui-inbox-unreadable", 0, panel.failedError, "")
                    textFormat: Text.PlainText
                    color: Theme.invalidText
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
                    // While the list can't be read, its rows are the previous read's: they are not shown under the error.
                    visible: panel.failedError.length === 0
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
                        color: Theme.statusText
                        font.bold: true
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
                                    color: Theme.statusText
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
                                        panel.folderRevealRequested(file.index);
                                    else
                                        panel.revealRequested(file.fileId);
                                }
                            }
                        }
                    }
                }
                Label {
                    visible: panel.failedTotal > files.count && !panel.failedLoading && panel.failedError.length === 0
                    // The scans' folders, or a pass's files.
                    text: panel.selectedPass === panel.scanFoldersCode ? words.phrase("ui-inbox-more-folders", 0, words.count(panel.failedTotal), words.count(files.count)) : words.phrase("ui-inbox-more", 0, words.count(panel.failedTotal), words.count(files.count))
                    textFormat: Text.PlainText
                    color: Theme.hintText
                }
            }
            RowLayout {
                // The action buttons are taken by Tab, not by a click: a click on one (which then hides or disables it) would
                // leave the keys with no receiver, where the list has them when the panel opens.
                Button {
                    objectName: "inboxRetry"
                    visible: panel.selectedCanRetry
                    focusPolicy: Qt.TabFocus
                    text: words.text("ui-inbox-retry")
                    onClicked: panel.retrySelected()
                }
                Button {
                    objectName: "inboxShowFiles"
                    visible: panel.selectedCanShowFiles
                    focusPolicy: Qt.TabFocus
                    text: words.text("ui-inbox-show-files")
                    onClicked: panel.showFilesRequested()
                }
                Button {
                    objectName: "inboxCopy"
                    text: words.text("ui-inbox-copy")
                    enabled: panel.selected >= 0
                    focusPolicy: Qt.TabFocus
                    onClicked: panel.copyRequested(panel.selected)
                }
                Button {
                    objectName: "inboxDismiss"
                    text: words.text("ui-inbox-dismiss")
                    enabled: panel.selected >= 0
                    focusPolicy: Qt.TabFocus
                    onClicked: panel.dismissSelected()
                }
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    objectName: "inboxClear"
                    text: words.text("ui-inbox-clear")
                    enabled: list.count > 0
                    focusPolicy: Qt.TabFocus
                    onClicked: {
                        panel.unselectRequested();
                        panel.clearRequested();
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
