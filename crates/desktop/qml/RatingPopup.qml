// The rating just given, for half a second (over the next picture). The root calls show() on each rating.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion

Label {
    id: popup

    anchors.centerIn: parent
    opacity: 0
    font.pixelSize: 120
    style: Text.Outline
    styleColor: "black"

    function show(r: int) {
        text = Ratings.symbol(r)
        fade.restart()
    }

    SequentialAnimation {
        id: fade
        PropertyAction { target: popup; property: "opacity"; value: 1 }
        PauseAnimation { duration: 350 }
        NumberAnimation { target: popup; property: "opacity"; to: 0; duration: 150 }
    }
}
