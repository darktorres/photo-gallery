// A mascot for the empty space under the folder tree, anchored to the bottom. A different one each time it is made
// (each time the window opens): resources/mascots/01.png to 40.png, registered by build.rs. A left click shows the next
// one, a right click the previous one, both wrapping around.
pragma ComponentBehavior: Bound

import QtQuick

Image {
    id: mascot
    // How many mascots there are: the files 01.png to 40.png in resources/mascots.
    readonly property int count: 40
    // The mascot shown: 1 to count.
    property int number: 1

    // Shows the mascot `d` places on (-1 the previous one, 1 the next), wrapping around.
    function step(d: int): void {
        number = (number - 1 + d + count) % count + 1
    }

    height: 200
    fillMode: Image.PreserveAspectFit
    horizontalAlignment: Image.AlignHCenter
    verticalAlignment: Image.AlignBottom
    asynchronous: true
    smooth: true
    mipmap: true
    source: "qrc:/qt/qml/pg/gallery/resources/mascots/" + (number < 10 ? "0" + number : "" + number) + ".png"

    Component.onCompleted: number = Math.floor(Math.random() * count) + 1

    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onClicked: (mouse) => mascot.step(mouse.button === Qt.RightButton ? -1 : 1)
    }
}
