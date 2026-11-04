// The test hooks: command-line options that drive the window without a person at the screen, for the screenshots and the
// checks of the viewer's keys, the views menu, the panels, the Yandex search and the library.
//   --size=<px>  row height   --select=<n>  select item n   --view=<n>  open item n in the viewer
//   --viewer-keys=<keys>  e.g. "LB" (loupe, borders); also H (zen), K (play/pause), + (zoom in), 1 (1:1), < > (turn
//                         left, right), N (not a border), digits after "r" rate
//   --zoom=<factor>  zoom the viewer to this many device px per image px
//   --menu=views  open the views menu   --apply-view=<name>  open a saved view
//   --query=<json>  filter by a query::Query   --sort=<json>  order by a query::Sort   --filters=1  open the bar
//   --library=1  open the library folders   --save-view=1  open the save-view dialog
//   --find-similar=<path>  search Yandex for the file (it is uploaded: a path that isn't a still image only shows
//                          the panel with its error)
//   --yandex-preview=<n>  with --find-similar: show result n at full size once the results are in
//   --add-folder=<path>  add a folder to the library (as "Add folder…" does)
//   --borders  show the borders (as B does) from the start
//   --save-crops=<folder>  save cropped copies of the listed files with borders found
//                        (GalleryModel.saveCroppedCopies)
//   --screenshot=<png>  save the window once it has settled (no image loading, no QML timer pending, no animation, no
//                       library job or write pending, no frame for a while), and quit; --wait=<s> is the most to wait
//                       for that, 10 by default (src/screenshot.rs)
// The options are read here. What they do to the window is signalled to Main, which makes the calls on the viewer and
// the Rust objects. --query, --sort, --apply-view, --filters and --borders shape the first list (Main reads
// them before it loads); the others act once the first list is shown (run, from started()).
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
    signal similarRequested(string path)
    signal addFolderRequested(url folder)
    signal saveCropsRequested(url folder)
    // The viewer's calls (its functions), made by Main.
    signal viewerShowRequested(int index)
    signal viewerPlayRequested()
    signal viewerSetZoomRequested(real zoom, real px, real py)
    signal viewerTurnRequested(int turns)
    signal viewerRateRequested(int rating)

    // The command line, read by this component: CommandLine is stateless (the arguments are read once).
    CommandLine { id: commandLine }

    // A property, read afresh at each key: a list held in a variable over calls isn't compiled.
    readonly property list<string> viewerKeys: commandLine.keys("viewer-keys")
    // The rating keys (--viewer-keys): "r3" rates 3.
    readonly property list<string> ratingKeys: ["r0", "r1", "r2", "r3", "r4", "r5"]

    readonly property bool hasQuery: commandLine.has("query")
    readonly property string queryJson: hasQuery ? commandLine.text("query") : ""
    readonly property bool hasSort: commandLine.has("sort")
    readonly property string sortJson: hasSort ? commandLine.text("sort") : ""
    readonly property bool hasApplyView: commandLine.has("apply-view")
    readonly property string applyViewName: hasApplyView ? commandLine.text("apply-view") : ""
    readonly property bool hasFilters: commandLine.has("filters")
    readonly property bool hasBorders: commandLine.has("borders")
    // The result the Yandex panel shows at full size (-1: none). It counts only with --find-similar, which makes the panel.
    readonly property int yandexPreview: commandLine.has("find-similar") && commandLine.has("yandex-preview")
        ? commandLine.number("yandex-preview", 0) : -1

    // The views menu opens half a second after the start, once the window is laid out.
    Timer {
        id: menuTimer
        interval: 500
        onTriggered: hooks.viewsMenuRequested()
    }

    // The hooks that run once the first list is shown (Main's started(), before the library job starts).
    function run() {
        if (commandLine.has("size")) sizeRequested(commandLine.number("size", rowHeight))
        if (commandLine.has("select")) selectRequested(commandLine.number("select", -1))
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
        if (commandLine.has("find-similar")) similarRequested(commandLine.text("find-similar"))
    }

    // The hooks that run after the library job has started (Main's started()).
    function runAfterLibraryStart() {
        if (commandLine.has("add-folder")) addFolderRequested(commandLine.url("add-folder"))
        if (commandLine.has("save-crops")) saveCropsRequested(commandLine.url("save-crops"))
    }
}
