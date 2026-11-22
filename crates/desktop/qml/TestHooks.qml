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
//   --dev=1  dev mode for the run, not saved (read by the shell, in Rust; PG_DEV=1 too; --dev=0 off for the run)
//   --log=1  open the log view (in dev mode)   --log-view=<path>  the log view shows that file, read once, following
//            nothing (read by the log view, in Rust)
//   --lane-rows=<path>  the status bar's lane rows are the file's, read once, not the job's (read by LaneModel, in Rust)
//   --inbox=1  open the inbox   --inbox-expand=<ledger>  open that ledger's entry in it (thumbs, borders, import,
//              scan-folders), when it is listed and again when it appears
//   --save-crops=<folder>  save cropped copies of the listed files with borders found
//                        (GalleryModel.saveCroppedCopiesPicked)
//   --screenshot=<png>  save the window once it has settled (no image loading, no QML timer pending, no animation, no
//                       library job or write pending, no frame for a while), and quit; --wait=<s> is the most to wait
//                       for that, 10 by default (src/screenshot.rs)
// The options are read in Rust, and so are the hooks they ask for and the order they run in (`test_hooks.rs`): the shell
// runs the selection, the grid's keys and the similar search itself, and hands each of the rest to `perform`, with the
// kind that `Kind` names. --query, --sort, --apply-view, --size, --filters, --borders and --dev shape the start in Rust
// (controller.rs); the others run once the first list is shown (Shell.runTestHooks), or once the library has started.
pragma ComponentBehavior: Bound

import QtQuick
import pg.gallery

Item {
    id: hooks
    // The viewer, which the viewer hooks drive.
    required property Viewer viewer

    // What a hook asks the window to do, in the order of the Rust `Kind` enum (test_hooks.rs), which is the same.
    enum Kind {
        ViewerShow,
        ViewerLoupe,
        Borders,
        BorderMark,
        ViewerZen,
        ViewerPlay,
        ZoomIn,
        ZoomOne,
        ZoomTo,
        Turn,
        Rate,
        ViewsMenu,
        LibraryPanel,
        SaveView,
        LogPanel,
        InboxExpand,
        InboxPanel,
        AddFolder,
        SaveCrops
    }

    // Done on the window by Main.
    signal borderMarkToggled(int index)
    signal bordersRequested
    signal viewsMenuRequested
    signal libraryPanelRequested
    signal saveViewDialogRequested
    signal addFolderRequested
    signal saveCropsRequested
    signal logPanelRequested
    signal inboxPanelRequested
    // The inbox's entry to open, by its ledger's code (--inbox-expand).
    signal inboxExpandRequested(string code)
    // The viewer's calls (its functions), made by Main.
    signal viewerShowRequested(int index)
    signal viewerPlayRequested
    signal viewerSetZoomRequested(real zoom, real px, real py)
    signal viewerTurnRequested(int turns)
    signal viewerRateRequested(int rating)

    // The options the window still reads itself, as typed properties of CommandLine (command_line.rs): a "not given" value
    // is -1 for an index or a mascot.
    CommandLine {
        id: commandLine
    }

    // The mascot the window shows from the start (--mascot): -1 when not given, which is a random one.
    readonly property int mascot: commandLine.mascot
    // The window is being photographed (--screenshot): its text carets are not drawn, since they blink.
    readonly property bool screenshot: commandLine.screenshot
    // The result the Yandex panel shows at full size (-1: none). It counts only with --find-similar, which makes the panel.
    readonly property int yandexPreview: commandLine.yandexPreview

    // Performs one hook the shell hands over (`testStep`): its kind (`Kind`), and the index, value or text it takes.
    // The viewer's own state is set here, and its calls go to the viewer through Main.
    function perform(kind: int, index: int, value: real, text: string) {
        switch (kind) {
        case TestHooks.Kind.ViewerShow:
            viewerShowRequested(index);
            break;
        case TestHooks.Kind.ViewerLoupe:
            viewer.loupe = true;
            break;
        case TestHooks.Kind.Borders:
            bordersRequested();
            break;
        case TestHooks.Kind.BorderMark:
            // -1 is the viewer's own picture.
            borderMarkToggled(index < 0 ? viewer.index : index);
            break;
        case TestHooks.Kind.ViewerZen:
            viewer.zen = true;
            break;
        case TestHooks.Kind.ViewerPlay:
            viewerPlayRequested();
            break;
        case TestHooks.Kind.ZoomIn:
            viewerSetZoomRequested(viewer.zoom * 1.25, viewer.width / 2, viewer.height / 2);
            break;
        case TestHooks.Kind.ZoomOne:
            viewerSetZoomRequested(1, viewer.width / 2, viewer.height / 2);
            break;
        case TestHooks.Kind.ZoomTo:
            viewerSetZoomRequested(value, viewer.width / 2, viewer.height / 2);
            break;
        case TestHooks.Kind.Turn:
            viewerTurnRequested(index);
            break;
        case TestHooks.Kind.Rate:
            viewerRateRequested(index);
            break;
        case TestHooks.Kind.ViewsMenu:
            viewsMenuRequested();
            break;
        case TestHooks.Kind.LibraryPanel:
            libraryPanelRequested();
            break;
        case TestHooks.Kind.SaveView:
            saveViewDialogRequested();
            break;
        case TestHooks.Kind.LogPanel:
            logPanelRequested();
            break;
        case TestHooks.Kind.InboxExpand:
            inboxExpandRequested(text);
            break;
        case TestHooks.Kind.InboxPanel:
            inboxPanelRequested();
            break;
        case TestHooks.Kind.AddFolder:
            addFolderRequested();
            break;
        case TestHooks.Kind.SaveCrops:
            saveCropsRequested();
            break;
        }
    }
}
