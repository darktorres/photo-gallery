// The colours of the components that are not the Main window's own: the Yandex panel and its parts, and the filter
// bar. Each name says what the colour is for; the values are the ones those components used as literals.
pragma Singleton
import QtQuick

QtObject {
    // The dimmed window behind the Yandex panel.
    readonly property color panelScrim: "#b0000000"
    // The full-size preview's background.
    readonly property color previewFill: "#141414"
    // A result tile's fill and its outline (the picked outline is the accent).
    readonly property color tileFill: "#202020"
    readonly property color tileOutline: "#444444"
    readonly property color pickedOutline: "#4a90e2"
    // A result tile's size label.
    readonly property color tileCaption: "#dddddd"
    // The panel's status line and the preview's size.
    readonly property color bodyText: "#cccccc"
    // Quiet text: "No results.", and a caption or a loading line.
    readonly property color hintText: "#888888"
    readonly property color captionText: "#999999"
    // A load that failed.
    readonly property color errorText: "#e57373"
    // A range field's text that doesn't parse.
    readonly property color invalidText: "#e06060"
}
