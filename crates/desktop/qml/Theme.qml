// The colours the components name, each written once: the Yandex panel and its parts, the filter bar, the status bar with
// its lane rows, the grid's labels, the inbox and log rows, and the scrims over pictures. Each name says what the colour is
// for; the values are the ones those components used as literals.
pragma Singleton
pragma ComponentBehavior: Bound
import QtQuick

QtObject {
    // The dimmed window behind the Yandex panel.
    readonly property color panelScrim: "#b0000000"
    // The full-size preview's background.
    readonly property color previewFill: "#141414"
    // A picked result's check badge (the accent) and the check on it.
    readonly property color pickedBadge: "#4a90e2"
    // A grid cell's selection outline, and a chosen row in a list (the window's palette highlight is the same blue).
    readonly property color selectionOutline: "#4aa3ff"
    readonly property color chosenRow: "#2f5f93"
    readonly property color pickedText: "#ffffff"
    // A result tile's size label, and the dark backing it sits on, so it reads over any picture.
    readonly property color tileCaption: "#ffffff"
    readonly property color captionBacking: "#b3000000"
    // The lighter dark backing behind a control over a picture: a rating bar, the video transport, a grid cell's badges.
    readonly property color scrimBacking: "#a0000000"
    // The panel's status line and the preview's size.
    readonly property color bodyText: "#cccccc"
    // Quiet text: "No results.", and a caption or a loading line.
    readonly property color hintText: "#888888"
    readonly property color captionText: "#999999"
    // A load that failed.
    readonly property color errorText: "#e57373"
    // A range field's text that doesn't parse.
    readonly property color invalidText: "#e06060"
    // The status bar's quiet text (the selection, the status line, a lane's status), and its warning text (a failed write,
    // a warning toast, the failed files' count).
    readonly property color statusText: "#aaaaaa"
    readonly property color statusWarning: "#e0a050"
    // The viewer's background behind its picture: black, so the bars of a letterboxed image read as part of it.
    readonly property color viewerFill: "#000000"
    // The viewer's frame around a picture while its borders are shown: a magenta line, so black bars inside it stand out.
    readonly property color borderFrame: "#ff00ff"
}
