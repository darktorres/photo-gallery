// GridRow (S-14.3): the cells follow the row's items, a settling view keeps the items it has (no cells are made for a
// view on its way), a new version reads the same items again, and a change of count makes new cells. The row's value
// (its row role) is made here from the list's properties: the items, the version and the cells' values, as the model
// gives them in one change. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "GridRow"
    // Items created under a non-visible TestCase are not visible, so the row would make no cells (see tst_wheellistview).
    visible: true
    when: windowShown

    // One value per cell, for the cells a span can show: GridRow reads each array by the cell's index.
    function perCell(valueOf) {
        return Array.from({ length: 16 }, (_, i) => valueOf(i))
    }

    Component {
        id: viewC
        ListView {
            id: list
            width: 400
            height: 300
            property point rowSpan: Qt.point(0, 2)
            property int rowVersion: 0
            property bool rowSettling: false
            // The thumbnails' tag: changing it changes every cell's URL, as a row's new items do.
            property int rowTag: 0
            model: 1
            delegate: GridRow {
                row: ({
                    first: list.rowSpan.x,
                    count: list.rowSpan.y,
                    version: list.rowVersion,
                    badges: tc.perCell(() => ""),
                    ratings: tc.perCell(() => 0),
                    bordersMarked: tc.perCell(() => false),
                    imageAspects: tc.perCell(() => 1.5),
                    aspects: tc.perCell(() => 1.5),
                    contentX: tc.perCell(() => 0),
                    contentY: tc.perCell(() => 0),
                    contentWidth: tc.perCell(() => 100),
                    contentHeight: tc.perCell(() => 100),
                    thumbUrls: tc.perCell((i) => "image://thumb/t" + list.rowTag + "k" + i + "/100"),
                    thumbhashUrls: tc.perCell((i) => "image://thumbhash/k" + i),
                })
                rowHeight: 100
                gap: 4
                showBorders: false
                current: -1
                settling: list.rowSettling
            }
        }
    }

    function rowOf(list) {
        const r = list.itemAtIndex(0)
        verify(r !== null, "the view made its row")
        return r
    }

    // The row's cells: its children that are cells (the Repeater's delegates; the Repeater is a child of the Row too).
    function cellsOf(r) {
        const all = r.children[0].children
        const cells = []
        for (let i = 0; i < all.length; i++) {
            if (all[i].itemThumbUrl !== undefined)
                cells.push(all[i])
        }
        return cells
    }

    function test_the_cells_follow_the_span() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        compare(r.firstItem, 0)
        compare(r.shown.first, 0)
        compare(r.shown.count, 2)
        list.rowSpan = Qt.point(3, 4)
        wait(20)
        compare(r.firstItem, 3)
        compare(r.shown.first, 3)
        compare(r.shown.count, 4)
    }

    // Each cell shows its own item, with that item's thumbnail.
    function test_each_cell_shows_its_own_item() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        list.rowSpan = Qt.point(3, 3)
        list.rowTag = 2
        wait(20)
        const cells = cellsOf(rowOf(list))
        compare(cells.length, 3)
        for (let i = 0; i < cells.length; i++) {
            compare(cells[i].item, 3 + i)
            compare(cells[i].itemThumbUrl, "image://thumb/t2k" + i + "/100")
        }
    }

    // The Repeater makes its cells anew when the count changes, so no cell is kept for another item.
    function test_a_count_change_makes_new_cells() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        const before = cellsOf(r)
        compare(before.length, 2)
        list.rowSpan = Qt.point(0, 3)
        wait(20)
        const after = cellsOf(r)
        compare(after.length, 3)
        verify(after[0] !== before[0], "the first cell is new")
        verify(after[1] !== before[1], "the second cell is new")
    }

    function test_a_settling_view_keeps_its_cells() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        list.rowSettling = true
        list.rowSpan = Qt.point(5, 3)
        wait(20)
        // Not made for a view on its way: the row keeps the items it has.
        compare(r.shown.first, 0)
        compare(r.shown.count, 2)
        list.rowSettling = false
        wait(20)
        compare(r.shown.first, 5)
        compare(r.shown.count, 3)
    }

    function test_a_new_version_reads_the_same_items_again() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        list.rowVersion = 7
        wait(20)
        compare(r.shown.version, 7)
        compare(r.shown.first, 0)
        compare(r.shown.count, 2)
    }

    // A span and new thumbnails are given as two property changes here, and the row's value is rebuilt for each: the
    // row ends up with its new items and their own thumbnails.
    function test_the_row_shows_its_new_items_with_their_thumbnails() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        list.rowSpan = Qt.point(2, 3)
        list.rowTag = 1
        wait(20)
        compare(r.shown.thumbUrls[0], "image://thumb/t1k0/100")
        compare(r.shown.thumbUrls[2], "image://thumb/t1k2/100")
        compare(r.shown.first, 2)
        compare(r.shown.count, 3)
    }

    function test_the_row_is_as_wide_as_the_view() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        compare(rowOf(list).width, list.width)
    }
}
