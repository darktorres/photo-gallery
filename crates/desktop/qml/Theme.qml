// The colours of the components that are not the Main window's own: the Yandex panel and its parts, the filter bar, and the
// status bar with its lane rows. Each name says what the colour is for; the values are the ones those components used as
// literals.
pragma Singleton
import QtQuick

QtObject {
    // The dimmed window behind the Yandex panel.
    readonly property color panelScrim: "#b0000000"
    // The full-size preview's background.
    readonly property color previewFill: "#141414"
    // A picked result's check badge (the accent) and the check on it.
    readonly property color pickedBadge: "#4a90e2"
    readonly property color pickedText: "#ffffff"
    // A result tile's size label, and the dark backing it sits on, so it reads over any picture.
    readonly property color tileCaption: "#ffffff"
    readonly property color captionBacking: "#b3000000"
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
}
