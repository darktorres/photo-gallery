// YandexPreview: a still has no animation player; an animated item gives its player the original's URL; a quarter turn
// swaps the picture's sides; a new item shows its still again until its own animation draws. The stubs stand in for the
// Rust types (the search, the players, the key map): the tests check what the preview does with them.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "YandexPreview"
    visible: true
    when: windowShown

    Component {
        id: previewC
        YandexPreview {
            width: 800
            height: 600
            dpr: 1
            otherCount: 2
            similarCount: 1
            imageUrl: "image://yandex/1/original"
            sizeText: ""
            picked: false
            prevUrl: ""
            nextUrl: ""
            animationUrl: ""
            mirrored: false
            turns: 0
        }
    }

    function test_a_still_has_no_player_url() {
        const p = createTemporaryObject(previewC, tc);
        verify(p !== null);
        const player = findChild(p, "animation");
        verify(player !== null);
        compare(player.url, "");
    }

    function test_an_animated_item_gives_its_player_the_original() {
        const p = createTemporaryObject(previewC, tc);
        verify(p !== null);
        const player = findChild(p, "animation");
        verify(player !== null);
        p.animationUrl = "image://yandex/1/original";
        compare(player.url, "image://yandex/1/original");
        p.animationUrl = "";
        compare(player.url, "");
    }

    function test_a_quarter_turn_swaps_the_sides() {
        const p = createTemporaryObject(previewC, tc);
        verify(p !== null);
        compare(p.swapped, false);
        p.turns = 1;
        compare(p.swapped, true);
        p.turns = 2;
        compare(p.swapped, false);
        p.turns = 3;
        compare(p.swapped, true);
    }

    function test_a_new_item_shows_its_still_again() {
        const p = createTemporaryObject(previewC, tc);
        verify(p !== null);
        p.framesShown = true;
        p.imageUrl = "image://yandex/2/original";
        compare(p.framesShown, false);
    }
}
