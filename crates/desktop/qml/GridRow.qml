// A row of the grid: the delegate logic of the list's rows, kept in QML because it is about ListView's delegate pooling.
// The row's values come from the model as one value (its row role): its items, its version, and the values its cells
// show. The cells are GridCell. A component with a name, so GalleryGrid.topItem() can read its firstItem typed (and
// final, so the QML compiler can trust it through the cast).
pragma ComponentBehavior: Bound

import QtQuick

Item {
    id: rowItem
    // The model's row (galleryModel's ROLE_ROW): the row's items (first, count), its version, and the values its cells
    // show, one list per field with an entry per item of the row. The model gives them in one change, so the row takes
    // them in one step: no value is in before another.
    required property var row
    // The row's height (galleryModel's ROLE_HEIGHT), made from the thumbnail height.
    required property real rowHeight
    // From the grid: the gap, the borders and selection, and whether the list is settling (see GalleryGrid).
    required property real gap
    required property bool showBorders
    required property int current
    required property bool settling

    final readonly property int firstItem: row ? row.first : 0
    width: ListView.view.width
    height: rowHeight

    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)

    // What the cells show: the row as it is once it is in use (not in the view's pool) and the list is not settling.
    // A view on its way makes no cells for a row it is passing, so a settling row keeps the items it has. A cell reads
    // its item's values by index, and reads them again whenever the row is taken.
    property var shown: null
    property bool pooled: false
    readonly property bool live: !pooled && !rowItem.settling
    // The number of cells: the shown row's count. When the row's items change place (its count or its first item), the
    // Repeater makes its cells anew (the count goes through 0 first, below), so no cell is kept for another item: a kept
    // cell would show the old item's image until the new one loads. A new version of the same items keeps the cells, and
    // each reads its item's values again.
    property int cellCount: 0

    function take() {
        if (!live || !row)
            return
        const moved = shown !== null && (shown.first !== row.first || shown.count !== row.count)
        if (moved || cellCount !== row.count)
            cellCount = 0
        shown = row
        cellCount = row.count
    }

    Component.onCompleted: take()
    onRowChanged: take()
    onLiveChanged: take()
    ListView.onPooled: pooled = true
    ListView.onReused: pooled = false

    Row {
        spacing: rowItem.gap
        Repeater {
            model: rowItem.cellCount
            delegate: GridCell {
                required property int index
                item: rowItem.shown.first + index
                // A cell the Repeater has taken away may outlive its removal for a moment, and then reads the new,
                // shorter lists past its index: it reads the empty value then, which nothing shows.
                itemBadge: rowItem.shown.badges[index] ?? ""
                itemRating: rowItem.shown.ratings[index] ?? 0
                itemBordersMarked: rowItem.shown.bordersMarked[index] ?? false
                itemImageAspect: rowItem.shown.imageAspects[index] ?? 1
                itemAspect: rowItem.shown.aspects[index] ?? 1
                itemContentX: rowItem.shown.contentX[index] ?? 0
                itemContentY: rowItem.shown.contentY[index] ?? 0
                itemContentWidth: rowItem.shown.contentWidth[index] ?? 0
                itemContentHeight: rowItem.shown.contentHeight[index] ?? 0
                version: rowItem.shown.version
                rowHeight: rowItem.rowHeight
                itemThumbUrl: rowItem.shown.thumbUrls[index] ?? ""
                itemThumbhashUrl: rowItem.shown.thumbhashUrls[index] ?? ""
                showBorders: rowItem.showBorders
                current: rowItem.current
                onCellClicked: (itemIndex) => rowItem.cellClicked(itemIndex)
                onCellDoubleClicked: (itemIndex) => rowItem.cellDoubleClicked(itemIndex)
            }
        }
    }
}
