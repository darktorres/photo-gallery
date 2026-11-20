// A row of the grid: the delegate logic of the list's rows, kept in QML because it is about ListView's delegate pooling.
// The row's values come from the model, one role per field: its items (first, count), the file each item is (ids), and
// the values its cells show, one list per field with an entry per item of the row. The cells are GridCell. A component
// with a name, so GalleryGrid.topItem() can read its firstItem typed (and final, so the QML compiler can trust it
// through the cast).
pragma ComponentBehavior: Bound

import QtQuick

Item {
    id: rowItem
    // The row's items: the first one, and their number.
    required property int first
    required property int count
    // The file each item is. A cell keeps its place only while it shows the same file in it (see take).
    required property list<real> ids
    // The values the cells show, one list per field (the model's roles), each with an entry per item of the row.
    required property list<string> thumbUrls
    required property list<string> thumbhashUrls
    required property list<string> badges
    required property list<int> ratings
    required property list<bool> bordersMarked
    required property list<real> imageAspects
    required property list<real> aspects
    required property list<real> contentX
    required property list<real> contentY
    required property list<real> contentWidth
    required property list<real> contentHeight
    // The row's height (the model's rowHeight role), made from the thumbnail height.
    required property real rowHeight
    // From the grid: the gap, the borders and selection, and whether the list is settling (see GalleryGrid).
    required property real gap
    required property bool showBorders
    required property int current
    required property bool settling
    // The row's place in the view (the delegate's index). The image loader serves queued requests newest first, so the
    // pictures are asked for one row at a time, from the top: a row asks once the row above has its pictures (aboveWaiting,
    // from GalleryGrid), and then goes on asking (loadStarted).
    final required property int index
    property bool aboveWaiting: false

    final readonly property int firstItem: rowItem.shownFirst
    width: ListView.view.width
    height: rowHeight

    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)

    // What the cells show: the row as it was taken, while it is live (not in the view's pool, and the list not settling).
    // A view on its way makes no cells for a row it is passing, so a settling row keeps the items it has. The model's
    // values reach the row one role at a time, each changing a property that takes the row again (`onXChanged` below),
    // and the last take, on the last of them, reads them all as they now are: no cell shows a mix of two rows.
    property int shownFirst: 0
    property int shownCount: 0
    property list<real> shownIds: []
    property list<string> shownThumbUrls: []
    property list<string> shownThumbhashUrls: []
    property list<string> shownBadges: []
    property list<int> shownRatings: []
    property list<bool> shownBordersMarked: []
    property list<real> shownImageAspects: []
    property list<real> shownAspects: []
    property list<real> shownContentX: []
    property list<real> shownContentY: []
    property list<real> shownContentWidth: []
    property list<real> shownContentHeight: []
    property bool pooled: false
    readonly property bool live: !rowItem.pooled && !rowItem.settling
    // The number of cells: the shown count (see take for when the cells are made anew).
    property int cellCount: 0
    // Whether the row's pictures are asked for: from the first time the row is not waiting behind the one above, until it
    // is pooled (a row made again starts its turn over). Once asked, it keeps asking, so a row made above it later does not
    // take its pictures away.
    property bool loadStarted: false
    readonly property bool loadTurn: !rowItem.pooled && !rowItem.aboveWaiting
    // The cells whose thumbnails are still to come (GridCell.thumbPending), and whether the row waits for any. A pooled row
    // waits for none.
    property int pendingCells: 0
    final readonly property bool pendingLoad: !rowItem.pooled && rowItem.cellCount > 0 && rowItem.pendingCells > 0

    function recountPending() {
        let waiting = 0;
        for (let i = 0; i < cells.count; ++i) {
            const cell = cells.itemAt(i) as GridCell;
            if (cell && cell.thumbPending)
                waiting++;
        }
        rowItem.pendingCells = waiting;
    }

    // Whether two rows' cells show the same files, in the same places.
    function sameFiles(a: list<real>, b: list<real>): bool {
        if (a.length !== b.length)
            return false;
        for (let i = 0; i < a.length; ++i) {
            if (a[i] !== b[i])
                return false;
        }
        return true;
    }

    function take() {
        if (!rowItem.live)
            return;
        // A cell keeps its place only while it shows the same file in it: a kept cell would show another file's image
        // until the new one loads. A row whose first item or files change makes new cells; a row given again with the
        // same files keeps them, and each cell reads its item's values again.
        if (rowItem.shownFirst !== rowItem.first || !rowItem.sameFiles(rowItem.shownIds, rowItem.ids))
            rowItem.cellCount = 0;
        rowItem.shownFirst = rowItem.first;
        rowItem.shownCount = rowItem.count;
        rowItem.shownIds = rowItem.ids;
        rowItem.shownThumbUrls = rowItem.thumbUrls;
        rowItem.shownThumbhashUrls = rowItem.thumbhashUrls;
        rowItem.shownBadges = rowItem.badges;
        rowItem.shownRatings = rowItem.ratings;
        rowItem.shownBordersMarked = rowItem.bordersMarked;
        rowItem.shownImageAspects = rowItem.imageAspects;
        rowItem.shownAspects = rowItem.aspects;
        rowItem.shownContentX = rowItem.contentX;
        rowItem.shownContentY = rowItem.contentY;
        rowItem.shownContentWidth = rowItem.contentWidth;
        rowItem.shownContentHeight = rowItem.contentHeight;
        rowItem.cellCount = rowItem.count;
    }

    Component.onCompleted: {
        take();
        if (rowItem.loadTurn)
            rowItem.loadStarted = true;
    }
    onLoadTurnChanged: if (rowItem.loadTurn)
        rowItem.loadStarted = true
    onLiveChanged: take()
    onFirstChanged: take()
    onCountChanged: take()
    onIdsChanged: take()
    onThumbUrlsChanged: take()
    onThumbhashUrlsChanged: take()
    onBadgesChanged: take()
    onRatingsChanged: take()
    onBordersMarkedChanged: take()
    onImageAspectsChanged: take()
    onAspectsChanged: take()
    onContentXChanged: take()
    onContentYChanged: take()
    onContentWidthChanged: take()
    onContentHeightChanged: take()
    ListView.onPooled: {
        rowItem.pooled = true;
        rowItem.loadStarted = false;
    }
    ListView.onReused: rowItem.pooled = false

    Row {
        spacing: rowItem.gap
        Repeater {
            id: cells
            model: rowItem.cellCount
            onItemAdded: rowItem.recountPending()
            onItemRemoved: rowItem.recountPending()
            delegate: GridCell {
                required property int index
                item: rowItem.shownFirst + index
                loadNow: rowItem.loadStarted
                onThumbPendingChanged: rowItem.recountPending()
                // A cell the Repeater has taken away may outlive its removal for a moment, and then reads the new,
                // shorter lists past its index: it reads the empty value then, which nothing shows.
                itemBadge: rowItem.shownBadges[index] ?? ""
                itemRating: rowItem.shownRatings[index] ?? 0
                itemBordersMarked: rowItem.shownBordersMarked[index] ?? false
                itemImageAspect: rowItem.shownImageAspects[index] ?? 1
                itemAspect: rowItem.shownAspects[index] ?? 1
                itemContentX: rowItem.shownContentX[index] ?? 0
                itemContentY: rowItem.shownContentY[index] ?? 0
                itemContentWidth: rowItem.shownContentWidth[index] ?? 0
                itemContentHeight: rowItem.shownContentHeight[index] ?? 0
                rowHeight: rowItem.rowHeight
                itemThumbUrl: rowItem.shownThumbUrls[index] ?? ""
                itemThumbhashUrl: rowItem.shownThumbhashUrls[index] ?? ""
                showBorders: rowItem.showBorders
                current: rowItem.current
                onCellClicked: itemIndex => rowItem.cellClicked(itemIndex)
                onCellDoubleClicked: itemIndex => rowItem.cellDoubleClicked(itemIndex)
            }
        }
    }
}
