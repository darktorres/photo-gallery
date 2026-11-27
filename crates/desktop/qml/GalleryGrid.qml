// The grid of the open list: the justified rows of the gallery model in a WheelListView, with its own scrollbar and the
// drag readout, and the message when the list is empty. Main owns the model and the start; this reports the clicks, the
// keys and the size steps (signals) and takes the calls it is asked for (settling, positionAt, takeFocus).
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import pg.gallery

Item {
    id: root
    Words {
        id: words
    }
    // The gallery model (made by Main, which also gives it the thumbnail height), the gap between cells, the selection
    // and the borders.
    required property GalleryModel gallery
    required property real gap
    required property int current
    required property bool showBorders
    // The empty-list message: no library folders, a filter that matches nothing, or an import under way.
    required property bool noFolders
    required property bool filtering
    required property bool importing

    // True while the list is reloaded, laid out again or positioned (set by Main.qml): its rows are made without their
    // cells until it is where it is going (see GridRow). Scrolling does not set it.
    property bool settling: false
    // The width the rows are laid out for (the list's, inside its margins).
    readonly property real gridWidth: grid.width

    signal keyPressed(event: KeyEvent)
    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)
    // The wheel with Ctrl: the row height is multiplied by `factor`.
    signal sizeStepped(real factor)
    // The list's width changed: the rows are laid out again.
    signal gridResized(real width)

    // The first item of the topmost visible row, to keep the view anchored across relayouts. A probe in the gap between
    // two rows finds no row, so the probe moves down past the gap.
    function topItem(): int {
        let row = grid.indexAt(10, grid.contentY + 1);
        if (row < 0)
            row = grid.indexAt(10, grid.contentY + 1 + grid.spacing);
        const del = row >= 0 ? grid.itemAtIndex(row) as GridRow : null;
        return del ? del.firstItem : -1;
    }

    // The scroll bar is being dragged: the view follows the drag, not the list's placement.
    readonly property bool dragging: gridBar.pressed

    // Bumped when a row's waiting, place or pooling changes: the rows read the row above again (waitsForAbove), as they
    // would not do on the row above's own change alone.
    property int pendingRevision: 0

    // Whether the row above `row` still waits for its pictures: a row asks for its own only after that one has them (see
    // GridRow). A row above that is not made (out of the view's cache) waits for nothing.
    function waitsForAbove(row: int): bool {
        const above = row > 0 ? grid.itemAtIndex(row - 1) as GridRow : null;
        return above !== null && above.pendingLoad;
    }

    function positionAt(row: int, mode: int) {
        grid.positionViewAtIndex(row, mode);
    }

    function positionAtBeginning() {
        grid.positionViewAtBeginning();
    }

    function takeFocus() {
        grid.forceActiveFocus();
    }

    WheelListView {
        id: grid
        anchors.fill: parent
        anchors.margins: root.gap
        // The scrollbar's lane: on the right of the grid, not over the rows (its 16 px, and a gap).
        anchors.rightMargin: root.gap + 16
        clip: true
        focus: true
        model: root.gallery
        spacing: root.gap
        cacheBuffer: 1500
        reuseItems: true
        boundsBehavior: Flickable.StopAtBounds
        onWidthChanged: root.gridResized(width)
        // Rows made or dropped by a change of the model: the rows above them are read again.
        onCountChanged: root.pendingRevision++
        Keys.onPressed: e => root.keyPressed(e)

        // Ctrl+wheel: a notch (120 units of angleDelta) is one step of 1.1, a finer wheel a part of one. A wheel with no
        // vertical turn (a sideways scroll) changes nothing.
        WheelHandler {
            acceptedModifiers: Qt.ControlModifier
            onWheel: ev => {
                if (ev.angleDelta.y !== 0)
                    root.sizeStepped(Math.pow(1.1, ev.angleDelta.y / 120));
            }
        }

        delegate: GridRow {
            gap: root.gap
            showBorders: root.showBorders
            current: root.current
            settling: root.settling
            aboveWaiting: root.pendingRevision >= 0 && root.waitsForAbove(index)
            onPendingLoadChanged: root.pendingRevision++
            onIndexChanged: root.pendingRevision++
            onPooledChanged: root.pendingRevision++
            Component.onCompleted: root.pendingRevision++
            onCellClicked: itemIndex => {
                root.cellClicked(itemIndex);
                grid.forceActiveFocus();
            }
            onCellDoubleClicked: itemIndex => root.cellDoubleClicked(itemIndex)
        }
    }

    // The empty-list message, or the error when the list could not be read: a failed read keeps the rows it had (the
    // notice says why), and an empty list that failed is not an empty folder.
    Label {
        textFormat: Text.PlainText
        anchors.centerIn: parent
        visible: root.gallery.count === 0 && !root.gallery.loading
        text: root.gallery.listingFailed ? words.text("ui-listing-failed") : root.noFolders ? words.text("ui-no-folders") : root.filtering ? words.text("ui-nothing-matches") : root.importing ? words.text("ui-nothing-importing") : words.text("ui-no-images")
        horizontalAlignment: Text.AlignHCenter
        color: root.gallery.listingFailed ? Theme.invalidText : "#888888"
    }

    // The grid's scrollbar, in its own lane to the right of the grid. It is a standalone bar, not the view's
    // attached one, which lies over the view's own right edge. Qt's style makes the handle a 6 px nub, as
    // long as it is wide at least: on a big library it is all but impossible to grab, so this one is 12 px
    // wide and at least 48 px long, and brightens as it is hovered and dragged.
    ScrollBar {
        id: gridBar
        orientation: Qt.Vertical
        policy: ScrollBar.AlwaysOn
        anchors.top: grid.top
        anchors.bottom: grid.bottom
        anchors.right: parent.right
        anchors.rightMargin: root.gap
        size: grid.visibleArea.heightRatio
        minimumSize: height > 0 ? Math.min(1, 48 / height) : 0
        // The view follows the bar while it is not dragged; a drag moves the view.
        Binding on position {
            value: grid.visibleArea.yPosition
            when: !gridBar.pressed
        }
        onPositionChanged: if (pressed)
            grid.contentY = grid.originY + position * grid.contentHeight
        contentItem: Rectangle {
            implicitWidth: 12
            radius: width / 2
            color: gridBar.pressed ? "#a0a0a0" : gridBar.hovered ? "#808080" : "#5a5a5a"
        }
    }

    // While the scrollbar is dragged: about where in the library the grid is, in files.
    Rectangle {
        visible: gridBar.pressed && root.gallery.count > 0
        width: readout.implicitWidth + 16
        height: readout.implicitHeight + 8
        x: parent.width - width - 24
        // Centred on the handle as it is drawn (visualPosition, visualSize: the bar's minimum length included); the number
        // below is the logical position's.
        y: Math.max(0, Math.min(parent.height - height, gridBar.y + (gridBar.visualPosition + gridBar.visualSize / 2) * gridBar.height - height / 2))
        radius: 4
        color: "#2a2a2a"
        border.color: "#555555"
        Label {
            id: readout
            textFormat: Text.PlainText
            anchors.centerIn: parent
            color: "#dddddd"
            font.pixelSize: 12
            text: {
                const f = gridBar.size < 1 ? Math.min(1, gridBar.position / (1 - gridBar.size)) : 0;
                return words.phrase("ui-grid-position", 0, words.count(Math.round(f * (root.gallery.count - 1)) + 1), words.count(root.gallery.count));
            }
        }
    }
}
