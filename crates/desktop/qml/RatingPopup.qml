// The rating just given, for half a second (over the next picture). The root calls show() on each rating.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion

Label {
    id: popup

    anchors.centerIn: parent
    opacity: 0
    // The rating shown: ☆ (no rating) is a symbol; the other ratings are emoji.
    property int rating: 0
    font.family: popup.rating === 0 ? Theme.symbolFamily : Theme.emojiFamily
    font.pixelSize: 120
    style: Text.Outline
    styleColor: "black"

    function show(r: int) {
        popup.rating = r;
        text = Ratings.symbol(r);
        fade.restart();
    }

    SequentialAnimation {
        id: fade
        PropertyAction {
            target: popup
            property: "opacity"
            value: 1
        }
        PauseAnimation {
            duration: 350
        }
        NumberAnimation {
            target: popup
            property: "opacity"
            to: 0
            duration: 150
        }
    }
}
