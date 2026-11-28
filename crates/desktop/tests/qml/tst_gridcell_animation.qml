// GridCell's animation: an animated file plays in its cell only while its row plays it, at the cell's height; a still
// has no player. The AnimationPlayer is a stub (its file and height are checked here, not its frames).
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "GridCellAnimation"
    visible: true
    when: windowShown

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
            itemFileId: 7
            itemAnimated: true
            itemPlayerTurns: 0
            itemPlayerMirrored: false
            itemPlays: true
            itemThumbUrl: ""
            itemThumbhashUrl: ""
            showBorders: false
            current: -1
        }
    }

    function test_an_animated_cell_plays_its_file_at_its_height() {
        const c = createTemporaryObject(cellC, tc);
        verify(c !== null);
        const player = findChild(c, "cellAnimation");
        verify(player !== null, "the cell makes its player");
        compare(player.fileId, 7);
        compare(player.height, 100);
    }

    function test_a_still_cell_has_no_player() {
        const c = createTemporaryObject(cellC, tc, { itemAnimated: false });
        verify(c !== null);
        compare(findChild(c, "cellAnimation"), null);
    }

    function test_a_cell_whose_row_may_not_play_has_none() {
        const c = createTemporaryObject(cellC, tc, { itemPlays: false });
        verify(c !== null);
        compare(findChild(c, "cellAnimation"), null);
    }
}
