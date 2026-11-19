// GridCell's error mark: a thumbnail that won't load shows one in its place, and a loaded thumbnail does not. The
// broken URL is one the test host's stub provider answers with no image (tests/qml_host/main.cpp). Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "GridCellWarning"
    // Items created under a non-visible TestCase are not visible (see tst_wheellistview).
    visible: true
    when: windowShown
    width: 200
    height: 100

    Component {
        id: cellC
        GridCell {
            item: 0
            itemBadge: ""
            itemRating: 0
            itemBordersMarked: false
            itemImageAspect: 1
            itemAspect: 1
            itemContentX: 0
            itemContentY: 0
            itemContentWidth: 0
            itemContentHeight: 0
            rowHeight: 100
            itemThumbhashUrl: "image://thumbhash/warning"
            showBorders: false
            current: -1
        }
    }

    function test_a_failed_thumbnail_shows_the_warning() {
        const cell = createTemporaryObject(cellC, tc, { itemThumbUrl: "image://thumb/broken/100" });
        verify(cell !== null);
        const thumb = findChild(cell, "thumb");
        tryCompare(thumb, "status", Image.Error);
        compare(findChild(cell, "failed").visible, true);
    }

    function test_a_loaded_thumbnail_shows_no_warning() {
        const cell = createTemporaryObject(cellC, tc, { itemThumbUrl: "image://thumb/warning/100" });
        verify(cell !== null);
        const thumb = findChild(cell, "thumb");
        tryCompare(thumb, "status", Image.Ready);
        compare(findChild(cell, "failed").visible, false);
    }
}
