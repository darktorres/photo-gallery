// GridRow (S-14.3): the cells follow the row's span, a settling view keeps the cells it has (no cells are made for a
// view on its way), and a new version reads the same items again. The list's model is one row, whose values the test
// sets on the list. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery
import "../../qml"

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
            model: 1
            delegate: GridRow {
                span: list.rowSpan
                rowHeight: 100
                version: list.rowVersion
                keys: tc.perCell((i) => "k" + i)
                badges: tc.perCell(() => "")
                ratings: tc.perCell(() => 0)
                bordersMarked: tc.perCell(() => false)
                imageAspects: tc.perCell(() => 1.5)
                aspects: tc.perCell(() => 1.5)
                contentX: tc.perCell(() => 0)
                contentY: tc.perCell(() => 0)
                contentWidth: tc.perCell(() => 100)
                contentHeight: tc.perCell(() => 100)
                thumbUrls: tc.perCell((i) => "image://thumb/k" + i + "/100")
                thumbhashUrls: tc.perCell((i) => "image://thumbhash/k" + i)
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

    function test_the_cells_follow_the_span() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        compare(r.firstItem, 0)
        compare(r.cellsFirst, 0)
        compare(r.cellsCount, 2)
        list.rowSpan = Qt.point(3, 4)
        wait(20)
        compare(r.firstItem, 3)
        compare(r.cellsFirst, 3)
        compare(r.cellsCount, 4)
    }

    function test_a_settling_view_keeps_its_cells() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        list.rowSettling = true
        list.rowSpan = Qt.point(5, 3)
        wait(20)
        // Not made for a view on its way: the cells stay as they were.
        compare(r.cellsFirst, 0)
        compare(r.cellsCount, 2)
        list.rowSettling = false
        wait(20)
        compare(r.cellsFirst, 5)
        compare(r.cellsCount, 3)
    }

    function test_a_new_version_reads_the_same_items_again() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        const r = rowOf(list)
        const before = r.cellsVersion
        list.rowVersion = 7
        wait(20)
        compare(r.cellsVersion, before + 1)
        compare(r.cellsFirst, 0)
        compare(r.cellsCount, 2)
    }

    function test_the_row_is_as_wide_as_the_view() {
        const list = createTemporaryObject(viewC, tc)
        verify(list !== null)
        wait(20)
        compare(rowOf(list).width, list.width)
    }
}
