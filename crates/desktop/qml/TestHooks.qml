// The test hooks: command-line options that drive the window without a person at the screen, for the screenshots and the
// checks of the viewer's keys, the views menu, the panels, the Yandex search and the library.
//   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
//   --viewer-keys=<keys>  e.g. "LB" (loupe, borders); also H (zen), K (play/pause), + (zoom in), 1 (1:1), < > (turn
//                         left, right), N (not a border), digits after "r" rate
//   --grid-keys=<keys>  send the grid these keys, in order: L, R, U, D (left, right, up, down; the grid's steps)
//   --zoom=<factor>  zoom the viewer to this many device px per image px
//   --menu=views  open the views menu   --apply-view=<name>  open a saved view (read by the start, in Rust)
//   --query=<json>  filter by a query::Query   --sort=<json>  order by a query::Sort (both read by the start, in Rust)
//   --filters=1  open the bar
//   --mascot=<n>  show this mascot (1 to 40) instead of a random one: the screenshots pin one
//   --library=1  open the library folders   --save-view=1  open the save-view dialog
//   --find-similar=<path>  search Yandex for the file (it is uploaded: a path that isn't a still image only shows
//                          the panel with its error)
//   --yandex-preview=<n>  with --find-similar: show result n at full size once the results are in
//   --add-folder=<path>  add a folder to the library (as "Add folder…" does)
//   --borders=1  show the borders (as B does) from the start
//   --dev=1  dev mode for the run, not saved (read by the shell, in Rust; PG_DEV=1 too)   --log=1  open the log view (in
//            dev mode)   --inbox=1  open the inbox
//   --save-crops=<folder>  save cropped copies of the listed files with borders found
//                        (GalleryModel.saveCroppedCopiesPicked)
//   --screenshot=<png>  save the window once it has settled (no image loading, no QML timer pending, no animation, no
//                       library job or write pending, no frame for a while), and quit; --wait=<s> is the most to wait
//                       for that, 10 by default (src/screenshot.rs)
// The options are read here. What they do to the window is signalled to Main, which makes the calls on the viewer and
// the Rust objects. --query, --sort and --apply-view shape the first list in Rust (listing::start); --filters and
// --borders=1 are shown from the start by Main; the others act once the first list is shown (run, from startHooks()).
pragma ComponentBehavior: Bound

import QtQuick
import pg.gallery

Item {
    id: hooks
    // The viewer, which the viewer hooks drive.
    required property Viewer viewer
    // The grid's row height (--size's default).
    required property real rowHeight

    // Done on the window by Main.
    signal sizeRequested(real height)
    signal selectRequested(int index)
    signal borderMarkToggled(int index)
    signal bordersRequested()
    signal viewsMenuRequested()
    signal libraryPanelRequested()
    signal saveViewDialogRequested()
    signal similarRequested()
    signal addFolderRequested()
    signal saveCropsRequested()
    signal logPanelRequested()
    signal inboxPanelRequested()
    // The viewer's calls (its functions), made by Main.
    signal viewerShowRequested(int index)
    signal viewerPlayRequested()
    signal viewerSetZoomRequested(real zoom, real px, real py)
    signal viewerTurnRequested(int turns)
    signal viewerRateRequested(int rating)
    // A key pressed on the grid, as the keyboard sends it (Shell.keyPressed).
    signal gridKeyRequested(int key)

    // The command line, read by this component: CommandLine is stateless (the arguments are read once).
    CommandLine { id: commandLine }

    // A property, read afresh at each key: a list held in a variable over calls isn't compiled.
    readonly property list<string> viewerKeys: commandLine.keys("viewer-keys")
    // The grid's keys (--grid-keys): L, R, U, D are left, right, up, down.
    readonly property list<string> gridKeys: commandLine.keys("grid-keys")
    // The rating keys (--viewer-keys): "r3" rates 3.
    readonly property list<string> ratingKeys: ["r0", "r1", "r2", "r3", "r4", "r5"]

    // --apply-view is read by the window's start (listing::start); the session it leaves is saved once the start is over.
    readonly property bool hasApplyView: commandLine.has("apply-view")
    readonly property bool hasFilters: commandLine.has("filters")
    readonly property bool hasBorders: commandLine.has("borders")
    // The mascot the window shows from the start (--mascot): 0 when not given, which is a random one.
    readonly property int mascot: commandLine.number("mascot", 0)
    // The window is being photographed (--screenshot): its text carets are not drawn, since they blink.
    readonly property bool screenshot: commandLine.has("screenshot")
    // The result the Yandex panel shows at full size (-1: none). It counts only with --find-similar, which makes the panel.
    readonly property int yandexPreview: commandLine.has("find-similar") && commandLine.has("yandex-preview")
        ? commandLine.number("yandex-preview", 0) : -1

    // The views menu opens half a second after the start, once the window is laid out.
    Timer {
        id: menuTimer
        interval: 500
        onTriggered: hooks.viewsMenuRequested()
    }

    // The hooks that run once the first list is shown (Main's startHooks(), before the library job starts).
    function run() {
        if (commandLine.has("size")) sizeRequested(commandLine.number("size", rowHeight))
        if (commandLine.has("select")) selectRequested(commandLine.number("select", -1))
        for (let k = 0; k < gridKeys.length; k++) {
            const c = gridKeys[k]
            if (c === "L") gridKeyRequested(Qt.Key_Left)
            else if (c === "R") gridKeyRequested(Qt.Key_Right)
            else if (c === "U") gridKeyRequested(Qt.Key_Up)
            else if (c === "D") gridKeyRequested(Qt.Key_Down)
        }
        if (commandLine.has("view")) {
            viewerShowRequested(commandLine.number("view", -1))
            for (let k = 0; k < viewerKeys.length; k++) {
                const c = viewerKeys[k]
                if (c === "L") viewer.loupe = true
                else if (c === "B") bordersRequested()
                else if (c === "N") borderMarkToggled(viewer.index)
                else if (c === "H") viewer.zen = true
                else if (c === "K") viewerPlayRequested()
                else if (c === "+") viewerSetZoomRequested(viewer.zoom * 1.25, viewer.width / 2, viewer.height / 2)
                else if (c === "1") viewerSetZoomRequested(1, viewer.width / 2, viewer.height / 2)
                else if (c === "<") viewerTurnRequested(-1)
                else if (c === ">") viewerTurnRequested(1)
                else if (ratingKeys.indexOf(c) >= 0) viewerRateRequested(ratingKeys.indexOf(c))
            }
            if (commandLine.has("zoom"))
                viewerSetZoomRequested(commandLine.number("zoom", 1), viewer.width / 2, viewer.height / 2)
        }
        if (commandLine.text("menu") === "views") menuTimer.start()
        if (commandLine.has("library")) libraryPanelRequested()
        if (commandLine.has("save-view")) saveViewDialogRequested()
        if (commandLine.has("find-similar")) similarRequested()
        if (commandLine.has("log")) logPanelRequested()
        if (commandLine.has("inbox")) inboxPanelRequested()
    }

    // The hooks that run after the library job has started (Main's startHooks()).
    function runAfterLibraryStart() {
        if (commandLine.has("add-folder")) addFolderRequested()
        if (commandLine.has("save-crops")) saveCropsRequested()
    }
}
