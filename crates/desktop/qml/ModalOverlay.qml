// The dimmed backdrop of a panel over the window: an item of the page (so window grabs include it) that takes every click
// and wheel. Escape, or a click on the backdrop, asks for the panel to be dismissed; the panel's own content is its children.
pragma ComponentBehavior: Bound

import QtQuick

Rectangle {
    id: overlay
    // Shown while true.
    property bool open: false
    // Escape, or a click on the backdrop: the owner closes the panel.
    signal dismissed()
    // Hidden after it was shown: the owner gives the keys back to the page.
    signal hidden()

    color: "#b0000000"
    visible: overlay.open
    // The keys go to the panel when it opens.
    Component.onCompleted: forceActiveFocus()
    onVisibleChanged: visible ? forceActiveFocus() : overlay.hidden()
    Keys.onEscapePressed: overlay.dismissed()

    MouseArea {
        anchors.fill: parent
        onClicked: overlay.dismissed()
        onWheel: (wheel) => wheel.accepted = true
    }
}
