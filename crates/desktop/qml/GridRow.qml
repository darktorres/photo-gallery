// A row of the grid: the delegate logic of the list's rows, kept in QML because it is about ListView's delegate pooling.
// The row's span, version and height come from the model's roles, and the values its cells show; the cells are
// GridCell. A component with a name, so GalleryGrid.topItem() can read its firstItem typed (and final, so the QML
// compiler can trust it through the cast).
pragma ComponentBehavior: Bound

import QtQuick

Item {
    id: rowItem
    // The model's roles: the row's items (span: first, count), its height, and its version (galleryModel's ROLE_SPAN,
    // ROLE_HEIGHT and ROLE_VERSION).
    required property point span
    required property real rowHeight
    // New whenever the row's items change in place: rated, turned, marked, rewritten, or another list.
    required property int version
    // The values the cells show, one list per field with an entry per item of the row (galleryModel's ROLE_BADGES to
    // ROLE_CONTENT_HEIGHT). They are copied into typed lists in update(), which the QML compiler can read.
    required property var badges
    required property var ratings
    required property var bordersMarked
    required property var imageAspects
    required property var aspects
    required property var contentX
    required property var contentY
    required property var contentWidth
    required property var contentHeight
    // The image URLs of the cells (ROLE_THUMB_URLS, ROLE_THUMBHASH_URLS): made by the model at the thumbnail height.
    required property var thumbUrls
    required property var thumbhashUrls
    // From the grid: the gap, the borders and selection, and whether the view is settling (moved, see GalleryGrid).
    required property real gap
    required property bool showBorders
    required property int current
    required property bool settling

    final readonly property int firstItem: span.x
    width: ListView.view.width
    height: rowHeight

    signal cellClicked(int itemIndex)
    signal cellDoubleClicked(int itemIndex)

    // What the cells show, made to match the row once it is in use (not in the view's pool) and the view is where it is
    // going (settling): its span's items, at its version, with their values as the row has them then. A reuse sets the
    // span and the version one after the other, before the row is in use. A span change makes the cells read their
    // items, so a version signalled with it is not read again. Either way the cells read their items (and ask for
    // their images) once.
    property int cellsFirst: 0
    property int cellsCount: 0
    // Bumped to read the same items again.
    property int cellsVersion: 0
    // The shown items' values, taken in update() with the span and the version.
    property list<string> shownBadges
    property list<int> shownRatings
    property list<bool> shownBordersMarked
    property list<real> shownImageAspects
    property list<real> shownAspects
    property list<real> shownContentX
    property list<real> shownContentY
    property list<real> shownContentWidth
    property list<real> shownContentHeight
    property list<string> shownThumbUrls
    property list<string> shownThumbhashUrls
    property int shownVersion: -1
    property bool pooled: false
    readonly property bool live: !pooled && !rowItem.settling

    function update() {
        if (!live)
            return
        // The values as the row has them now: the model signals them before its span and version.
        shownBadges = badges
        shownRatings = ratings
        shownBordersMarked = bordersMarked
        shownImageAspects = imageAspects
        shownAspects = aspects
        shownContentX = contentX
        shownContentY = contentY
        shownContentWidth = contentWidth
        shownContentHeight = contentHeight
        shownThumbUrls = thumbUrls
        shownThumbhashUrls = thumbhashUrls
        const first = span.x
        const count = span.y
        if (cellsFirst !== first || cellsCount !== count) {
            // The Repeater (an integer model) makes all its cells anew when its count changes: they are made once,
            // for the new items.
            if (cellsCount !== count)
                cellsCount = 0
            cellsFirst = first
            cellsCount = count
        } else if (shownVersion !== version) {
            cellsVersion++
        }
        shownVersion = version
    }

    Component.onCompleted: update()
    onLiveChanged: update()
    onSpanChanged: update()
    onVersionChanged: update()
    ListView.onPooled: pooled = true
    ListView.onReused: pooled = false

    Row {
        spacing: rowItem.gap
        Repeater {
            model: rowItem.cellsCount
            delegate: GridCell {
                required property int index
                item: rowItem.cellsFirst + index
                itemBadge: rowItem.shownBadges[index]
                itemRating: rowItem.shownRatings[index]
                itemBordersMarked: rowItem.shownBordersMarked[index]
                itemImageAspect: rowItem.shownImageAspects[index]
                itemAspect: rowItem.shownAspects[index]
                itemContentX: rowItem.shownContentX[index]
                itemContentY: rowItem.shownContentY[index]
                itemContentWidth: rowItem.shownContentWidth[index]
                itemContentHeight: rowItem.shownContentHeight[index]
                version: rowItem.cellsVersion
                rowHeight: rowItem.rowHeight
                itemThumbUrl: rowItem.shownThumbUrls[index]
                itemThumbhashUrl: rowItem.shownThumbhashUrls[index]
                showBorders: rowItem.showBorders
                current: rowItem.current
                onCellClicked: (itemIndex) => rowItem.cellClicked(itemIndex)
                onCellDoubleClicked: (itemIndex) => rowItem.cellDoubleClicked(itemIndex)
            }
        }
    }
}
