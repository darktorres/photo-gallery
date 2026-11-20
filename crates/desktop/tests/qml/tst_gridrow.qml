// GridRow: the cells follow the row's items, a settling view keeps the items it has (no cells are made for a
// view on its way, nor for a row made while settling), and a change of items makes new cells where their places change.
// The row's values are set here as the model's roles set them: one property per field, each a change that takes the row
// again. Run by `just qmltest`.
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
        return Array.from({
            length: 16
        }, (_, i) => valueOf(i));
    }

    // The files of a row: `count` consecutive files from `first`. `again` is the model's signal for a row given again:
    // a new array with the same files.
    function filesOf(count, first, again) {
        return Array.from({
            length: count
        }, (_, i) => first + i);
    }

    Component {
        id: viewC
        ListView {
            id: list
            width: 400
            height: 300
            property point rowSpan: Qt.point(0, 2)
            property bool rowSettling: false
            // The file each cell shows: changing it swaps the files in place, as a refresh of the list can.
            property int rowFiles: 100
            // The thumbnails' tag: changing it changes every cell's URL, as a row's new items do.
            property int rowTag: 0
            // Changing it gives the row its files again (new arrays, the same files).
            property int rowAgain: 0
            // Whether the row above is still waiting for its pictures (GridRow.aboveWaiting): the default lets the row ask.
            property bool rowAboveWaiting: false
            model: 1
            delegate: GridRow {
                first: list.rowSpan.x
                count: list.rowSpan.y
                ids: tc.filesOf(list.rowSpan.y, list.rowFiles, list.rowAgain)
                badges: tc.perCell(() => "")
                ratings: tc.perCell(() => 0)
                bordersMarked: tc.perCell(() => false)
                imageAspects: tc.perCell(() => 1.5)
                aspects: tc.perCell(() => 1.5)
                contentX: tc.perCell(() => 0)
                contentY: tc.perCell(() => 0)
                contentWidth: tc.perCell(() => 100)
                contentHeight: tc.perCell(() => 100)
                thumbUrls: tc.perCell(i => "image://thumb/t" + list.rowTag + "k" + i + "/100")
                thumbhashUrls: tc.perCell(i => "image://thumbhash/k" + i)
                rowHeight: 100
                gap: 4
                showBorders: false
                current: -1
                settling: list.rowSettling
                aboveWaiting: list.rowAboveWaiting
            }
        }
    }

    function rowOf(list) {
        const r = list.itemAtIndex(0);
        verify(r !== null, "the view made its row");
        return r;
    }

    // The row's cells: its children that are cells (the Repeater's delegates; the Repeater is a child of the Row too).
    function cellsOf(r) {
        const all = r.children[0].children;
        const cells = [];
        for (let i = 0; i < all.length; i++) {
            if (all[i].itemThumbUrl !== undefined)
                cells.push(all[i]);
        }
        return cells;
    }

    function test_the_cells_follow_the_span() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        compare(r.firstItem, 0);
        compare(r.shownFirst, 0);
        compare(r.shownCount, 2);
        list.rowSpan = Qt.point(3, 4);
        wait(20);
        compare(r.firstItem, 3);
        compare(r.shownFirst, 3);
        compare(r.shownCount, 4);
    }

    // Each cell shows its own item, with that item's thumbnail.
    function test_each_cell_shows_its_own_item() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        list.rowSpan = Qt.point(3, 3);
        list.rowTag = 2;
        wait(20);
        const cells = cellsOf(rowOf(list));
        compare(cells.length, 3);
        for (let i = 0; i < cells.length; i++) {
            compare(cells[i].item, 3 + i);
            compare(cells[i].itemThumbUrl, "image://thumb/t2k" + i + "/100");
        }
    }

    // The Repeater makes its cells anew when the count changes, so no cell is kept for another item.
    function test_a_count_change_makes_new_cells() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        const before = cellsOf(r);
        compare(before.length, 2);
        list.rowSpan = Qt.point(0, 3);
        wait(20);
        const after = cellsOf(r);
        compare(after.length, 3);
        verify(after[0] !== before[0], "the first cell is new");
        verify(after[1] !== before[1], "the second cell is new");
    }

    // A span that moves the items along (the same count) makes new cells too: the cells would show another item's image.
    function test_a_span_change_makes_new_cells() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        const before = cellsOf(r);
        list.rowSpan = Qt.point(1, 2);
        wait(20);
        const after = cellsOf(r);
        compare(after.length, 2);
        verify(after[0] !== before[0], "the first cell is new");
        verify(after[1] !== before[1], "the second cell is new");
        compare(after[0].item, 1);
        compare(after[1].item, 2);
    }

    // A row behind a row still waiting for its pictures asks for none; once that row has them, it asks, and then keeps
    // asking, so a waiting row made above it later takes nothing away.
    function test_a_row_behind_a_waiting_row_asks_once_it_is_clear() {
        const list = createTemporaryObject(viewC, tc, {
            rowAboveWaiting: true
        });
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        compare(r.loadStarted, false);
        compare(r.pendingLoad, true, "its pictures are still to come");
        const cells = cellsOf(r);
        compare(cells[0].loadNow, false);
        compare(cells[0].thumbPending, true);
        list.rowAboveWaiting = false;
        wait(20);
        compare(r.loadStarted, true);
        compare(cells[0].loadNow, true);
        list.rowAboveWaiting = true;
        wait(20);
        compare(r.loadStarted, true, "a row above that waits again takes nothing away");
    }

    function test_a_settling_view_keeps_its_cells() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        list.rowSettling = true;
        list.rowSpan = Qt.point(5, 3);
        wait(20);
        // Not made for a view on its way: the row keeps the items it has.
        compare(r.shownFirst, 0);
        compare(r.shownCount, 2);
        list.rowSettling = false;
        wait(20);
        compare(r.shownFirst, 5);
        compare(r.shownCount, 3);
    }

    // A row given again with the same items (the model signals it again, without a change of span) keeps its cells, and
    // each still shows its item's values.
    function test_a_row_given_again_keeps_its_cells() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        const before = cellsOf(r);
        list.rowAgain = 1;
        wait(20);
        const after = cellsOf(r);
        compare(after.length, 2);
        verify(after[0] === before[0], "the first cell is kept");
        verify(after[1] === before[1], "the second cell is kept");
        compare(after[1].item, 1);
        compare(after[1].itemThumbUrl, "image://thumb/t0k1/100");
    }

    // A refresh that swaps a file for another in the same place (the same count and span) makes new cells: the old
    // cell would show the old file's image.
    function test_a_swapped_file_makes_new_cells() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        const before = cellsOf(r);
        list.rowFiles = 200;
        wait(20);
        const after = cellsOf(r);
        compare(after.length, 2);
        verify(after[0] !== before[0], "the first cell is new");
        verify(after[1] !== before[1], "the second cell is new");
        compare(after[1].itemThumbUrl, "image://thumb/t0k1/100");
    }

    // A kept cell takes its item's new values (here a new thumbnail tag), as the row's cells do when they are read again.
    function test_a_kept_cell_takes_its_new_values() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        const before = cellsOf(r);
        list.rowTag = 5;
        wait(20);
        const after = cellsOf(r);
        verify(after[0] === before[0], "the first cell is kept");
        compare(after[0].itemThumbUrl, "image://thumb/t5k0/100");
        compare(after[1].itemThumbUrl, "image://thumb/t5k1/100");
    }

    // A row made while its list is settling takes no cells until the list is settled.
    function test_a_row_made_while_settling_makes_no_cells() {
        const list = createTemporaryObject(viewC, tc, {
            rowSettling: true
        });
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        compare(cellsOf(r).length, 0);
        compare(r.shownCount, 0);
        list.rowSettling = false;
        wait(20);
        compare(cellsOf(r).length, 2);
        compare(r.shownCount, 2);
    }

    // A span and new thumbnails are given as two property changes here: the row ends up with its new items and their
    // own thumbnails.
    function test_the_row_shows_its_new_items_with_their_thumbnails() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        const r = rowOf(list);
        list.rowSpan = Qt.point(2, 3);
        list.rowTag = 1;
        wait(20);
        compare(r.shownThumbUrls[0], "image://thumb/t1k0/100");
        compare(r.shownThumbUrls[2], "image://thumb/t1k2/100");
        compare(r.shownFirst, 2);
        compare(r.shownCount, 3);
    }

    function test_the_row_is_as_wide_as_the_view() {
        const list = createTemporaryObject(viewC, tc);
        verify(list !== null);
        wait(20);
        compare(rowOf(list).width, list.width);
    }

    // Randomized: a view of 40 rows, pooled and reused as it scrolls, each row given random spans, counts, files,
    // thumbnail tags and settling. After each operation, every visible row (when the list is not settling) shows its
    // current items: its cell count, each cell's item, and each cell's own thumbnail.
    Component {
        id: fuzzC
        ListView {
            id: fz
            width: 400
            height: 300
            cacheBuffer: 200
            reuseItems: true
            // The rows' state: each row's first item, count, files and thumbnail tag (set before the rows are made, so no
            // row's binding reads a missing state).
            property var rows: Array.from({
                length: 40
            }, (_, i) => ({
                        first: i * 3,
                        count: 1 + (i % 4),
                        file: 1000 + i * 10,
                        tag: 0
                    }))
            property bool rowSettling: false
            // What each visible row showed at the last check: its delegate, the state's key, and its cells.
            property var lastRecords: []
            model: 40
            delegate: GridRow {
                readonly property var st: fz.rows[index]
                first: st.first
                count: st.count
                ids: Array.from({
                    length: st.count
                }, (_, i) => st.file + i)
                badges: Array.from({
                    length: st.count
                }, () => "")
                ratings: Array.from({
                    length: st.count
                }, () => 0)
                bordersMarked: Array.from({
                    length: st.count
                }, () => false)
                imageAspects: Array.from({
                    length: st.count
                }, () => 1.5)
                aspects: Array.from({
                    length: st.count
                }, () => 1.5)
                contentX: Array.from({
                    length: st.count
                }, () => 0)
                contentY: Array.from({
                    length: st.count
                }, () => 0)
                contentWidth: Array.from({
                    length: st.count
                }, () => 100)
                contentHeight: Array.from({
                    length: st.count
                }, () => 100)
                thumbUrls: Array.from({
                    length: st.count
                }, (_, i) => "image://thumb/t" + st.tag + "k" + (st.file + i) + "/100")
                thumbhashUrls: Array.from({
                    length: st.count
                }, (_, i) => "image://thumbhash/k" + (st.file + i))
                rowHeight: 100
                gap: 4
                showBorders: false
                current: -1
                settling: fz.rowSettling
            }
        }
    }

    // The visible rows of a fuzz view: the rows whose delegates are on screen, with their cells.
    function visibleRows(list) {
        const out = [];
        const kids = list.contentItem.children;
        for (let i = 0; i < kids.length; i++) {
            const d = kids[i];
            if (d.firstItem === undefined || !d.visible)
                continue;
            if (d.y + d.height > list.contentY && d.y < list.contentY + list.height)
                out.push(d);
        }
        return out;
    }

    function checkFuzzView(list, label) {
        const rows = visibleRows(list);
        verify(rows.length > 0, label + ": some rows are on screen");
        const records = [];
        for (const d of rows) {
            const st = list.rows[d.index];
            compare(d.firstItem, st.first, label + ": row " + d.index + " shows its first item");
            const cells = cellsOf(d);
            compare(cells.length, st.count, label + ": row " + d.index + " has its count of cells");
            for (let i = 0; i < cells.length; i++) {
                compare(cells[i].item, st.first + i, label + ": row " + d.index + " cell " + i + " shows its item");
                compare(cells[i].itemThumbUrl, "image://thumb/t" + st.tag + "k" + (st.file + i) + "/100", label + ": row " + d.index + " cell " + i + " shows its own thumbnail");
            }
            records.push({
                delegate: d,
                key: st.first + ":" + st.count + ":" + st.file,
                cells: cells
            });
        }
        // A cell kept from the last check, for a delegate whose items changed since (another first item, count or
        // files): its image would be the old file's until the new one loads, so it must have been made anew.
        for (const now of records) {
            const before = list.lastRecords.find(p => p.delegate === now.delegate);
            if (before === undefined || before.key === now.key)
                continue;
            for (const cell of now.cells)
                verify(before.cells.indexOf(cell) < 0, label + ": a cell was kept for another file in row " + now.delegate.index);
        }
        list.lastRecords = records;
    }

    function test_random_operations_keep_every_visible_row_right() {
        for (let seed = 1; seed <= 3; seed++) {
            const list = createTemporaryObject(fuzzC, tc);
            verify(list !== null);
            wait(20);
            let s = seed;
            const rnd = n => {
                s = (s * 1103515245 + 12345) & 0x7fffffff;
                return s % n;
            };
            list.rows = Array.from({
                length: 40
            }, (_, i) => ({
                        first: i * 3,
                        count: 1 + (i % 4),
                        file: 1000 + i * 10,
                        tag: 0
                    }));
            wait(20);
            for (let op = 0; op < 120; op++) {
                const label = "seed " + seed + " op " + op;
                const kind = rnd(6);
                const r = rnd(40);
                const rows = list.rows.slice();
                const row = Object.assign({}, rows[r]);
                if (kind === 0) {
                    row.first = rnd(100);
                    row.count = 1 + rnd(5);
                } else if (kind === 1) {
                    row.file = 1000 + rnd(400);
                } else if (kind === 2) {
                    row.tag = rnd(9);
                } else if (kind === 3) {
                    list.positionViewAtIndex(rnd(40), ListView.Beginning);
                } else if (kind === 4) {
                    list.rowSettling = !list.rowSettling;
                }
                if (kind < 3) {
                    rows[r] = row;
                    list.rows = rows;
                }
                wait(10);
                if (!list.rowSettling)
                    checkFuzzView(list, label);
            }
            list.rowSettling = false;
            wait(20);
            checkFuzzView(list, "seed " + seed + " end");
        }
    }
}
