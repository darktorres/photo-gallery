// TurnedVideoOutput: the picture's corners land where the file's orientation puts them (the mirror left-right first, then
// the quarter turns clockwise, as Orientation::parts says). A square box, so the sides do not swap. The VideoOutput has no
// frame here: its geometry is what is checked, through where its top-left corner maps in the test window.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "TurnedVideoOutput"
    visible: true
    when: windowShown

    Component {
        id: turnedC
        TurnedVideoOutput {
            width: 100
            height: 100
            swapsAxes: false
        }
    }

    // Where the picture's top-left corner is, in the test window, as [x, y].
    function corner(turned) {
        const p = turned.output.mapToItem(tc, 0, 0);
        return [Math.round(p.x), Math.round(p.y)];
    }

    function test_the_turns_go_clockwise_from_the_top_left() {
        const t = createTemporaryObject(turnedC, tc, { turns: 0, mirrored: false });
        verify(t !== null);
        compare(corner(t), [0, 0]);
        t.turns = 1;
        compare(corner(t), [100, 0]);
        t.turns = 2;
        compare(corner(t), [100, 100]);
        t.turns = 3;
        compare(corner(t), [0, 100]);
    }

    function test_a_mirror_moves_the_corner_across_before_the_turns() {
        const t = createTemporaryObject(turnedC, tc, { turns: 0, mirrored: true });
        verify(t !== null);
        // Mirror only: the top-left goes to the top-right.
        compare(corner(t), [100, 0]);
        // Mirror, then one turn: top-left → top-right (mirror) → bottom-right (turn).
        t.turns = 1;
        compare(corner(t), [100, 100]);
        // Mirror, then three turns: top-left → top-right → top-left.
        t.turns = 3;
        compare(corner(t), [0, 0]);
    }

    function test_the_mirror_is_off_again() {
        const t = createTemporaryObject(turnedC, tc, { turns: 1, mirrored: true });
        verify(t !== null);
        compare(corner(t), [100, 100]);
        t.mirrored = false;
        compare(corner(t), [100, 0]);
    }
}
