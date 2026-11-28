// ResultTile: an animated result plays only while the tile is shown, on screen and the panel lets tiles play; a still never
// does. The tile's values are the model's roles and the panel's scroll, set here as the panel sets them.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "ResultTile"
    visible: true
    when: windowShown

    Component {
        id: tileC
        ResultTile {
            width: 100
            height: 100
            index: 0
            picked: false
            thumb: ""
            imageWidth: 4
            imageHeight: 4
            frameX: 0
            frameY: 0
            frameWidth: 100
            frameHeight: 100
            isOther: true
            loadTurn: 10
            animationUrl: "image://yandex/1/original"
            mirrored: false
            turns: 0
            mayPlay: true
            blockY: 0
            viewTop: 0
            viewHeight: 300
        }
    }

    function test_an_animated_tile_on_screen_plays() {
        const t = createTemporaryObject(tileC, tc);
        verify(t !== null);
        compare(t.playing, true);
    }

    function test_a_still_does_not_play() {
        const t = createTemporaryObject(tileC, tc, { animationUrl: "" });
        verify(t !== null);
        compare(t.playing, false);
    }

    function test_a_tile_off_screen_does_not_play_until_it_comes_on() {
        const t = createTemporaryObject(tileC, tc, { blockY: 900 });
        verify(t !== null);
        compare(t.playing, false);
        // The list scrolls it into view: it starts.
        t.viewTop = 850;
        compare(t.playing, true);
        // And out again: it stops.
        t.viewTop = 0;
        compare(t.playing, false);
    }

    function test_a_moving_list_or_an_open_preview_stops_every_tile() {
        const t = createTemporaryObject(tileC, tc, { mayPlay: false });
        verify(t !== null);
        compare(t.playing, false);
        t.mayPlay = true;
        compare(t.playing, true);
    }

    function test_a_hidden_tile_does_not_play() {
        const t = createTemporaryObject(tileC, tc, { visible: false });
        verify(t !== null);
        compare(t.playing, false);
    }
}
