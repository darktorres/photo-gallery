// The colours the components name, each written once: the Yandex panel and its parts, the filter bar, the status bar with
// its lane rows, the grid's labels, the inbox and log rows, and the scrims over pictures. Each name says what the colour is
// for; the values are the ones those components used as literals. And the fonts of the texts that show symbols and emoji.
pragma Singleton
pragma ComponentBehavior: Bound
import QtQuick

QtObject {
    // The fonts that keep Qt from building its font fallback list. The first time a text needs a glyph its own font lacks,
    // Qt loads every installed font family to build that list: once per process, some 370 ms on the GUI thread, before
    // the first frame when the startup window needs it. Two things need it:
    // - a symbol the window's font (Segoe UI on Windows) lacks: ▾ ▸ ▴ ▭ ▶ ⟲ ⟳ ✓ ✕ ☆. Segoe UI Symbol has each, and its Latin
    //   letters are Segoe UI's, so a whole label of words can use it. It has no bold: a bold label with words keeps the
    //   window's font and a symbol Segoe UI has (the grid's ► badge).
    // - an emoji (the ratings, ⚠️ ⏸️ 🔇 🔊): Qt draws one from a colour font, and the text's own font is searched first.
    //   Segoe UI Emoji is one; with any other font Qt searches the fallback list for one. An emoji's text is the emoji
    //   alone, in this font: it lacks most of the letters. ⚠ and ⏸ carry VS16 (U+FE0F), which asks for the emoji.
    // A family list (font.families) would not help: Qt builds the fallback list for one at once. Where a family isn't
    // installed (Linux), Qt draws with the system's substitute for it (fontconfig's), from that fallback list
    // (QFontDatabasePrivate::findFont, its fallbacks pass). docs/plans/FONT_FALLBACK_PLAN.md is the fix in Qt.
    readonly property string symbolFamily: "Segoe UI Symbol"
    readonly property string emojiFamily: "Segoe UI Emoji"
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
    // The viewer's key hint over a picture: each key's chip (its fill and border), the key in it and the label beside it.
    readonly property color chipFill: "#d9101010"
    readonly property color chipBorder: "#555555"
    readonly property color chipKeyText: "#ffffff"
    readonly property color chipLabelText: "#bbbbbb"
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
    // The cut borders over a picture, and a grid cell's border mark: magenta, and grey where the borders are marked as not a border.
    readonly property color borderTint: "#ff00c8"
    readonly property color borderMarkedTint: "#a0a0a0"
}
