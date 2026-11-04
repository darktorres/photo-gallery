// The filter bar, under the top bar: one control per facet of the query (Filters, src/filters.rs, editing a
// query::Query). Facets combine with AND, the choices inside one with OR, and an empty facet doesn't filter. Each
// type, extension and borders choice shows how many files choosing it would show given the other facets
// (countFacets()). Edits go straight to the query, and edited() fires; the window applies them after a short pause.
//
// The window sees the query only through this bar's properties, signal and functions: the QML compiler compiles a
// call into a Rust object to C++ only where the object is named by its id, in its own file.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

Pane {
    id: bar

    // The query as JSON (query::Query), for the grid's model and the saved views; set it to replace the query.
    property alias query: filters.query
    // How many facets filter; files in the folder, unfiltered, and those the query keeps (countFacets()).
    readonly property int active: filters.active
    readonly property int total: filters.total
    readonly property int matching: filters.matching
    // What the query keeps, in one line.
    readonly property string description: filters.description

    // The user changed the query.
    signal edited()
    // Listed files with borders to crop off (GalleryModel.borderedCount), and the user asking to save them cropped.
    property int croppable: 0
    signal saveCropsRequested()

    // Count a merged folder's files (`""` = all folders), with or without its subfolders, for the counts.
    function countFacets(folder: string, recursive: bool) { filters.countFacets(folder, recursive) }

    // The choices of the list facets: the Rust enums, in the bar's order (the labels are Filters.label()).
    readonly property list<int> types: [Filters.MediaType.Image, Filters.MediaType.Animated, Filters.MediaType.Video]
    readonly property list<int> orientations: [Filters.Shape.Landscape, Filters.Shape.Portrait, Filters.Shape.Square]
    readonly property list<int> statuses: [Filters.Status.Imported, Filters.Status.Pending, Filters.Status.Failed]
    // Filters.duplicates: -1 either way, 1 with a duplicate, 0 without; in the duplicates box's order.
    readonly property list<string> duplicateChoices: ["Any", "Has duplicates", "No duplicates"]
    // query::BorderFacet, in the bar's order.
    readonly property list<int> borderValues: [
        Filters.Border.None, Filters.Border.Black, Filters.Border.White, Filters.Border.Color, Filters.Border.Blur
    ]
    // Filters.borderMarked in the review box's order (-1, 0, 1).
    readonly property list<string> reviewChoices: ["Any", "Not marked", "Marked not a border"]

    padding: 6

    // Shown once: the controls are made then (the window starts with the bar closed), and kept.
    property bool opened: false
    onVisibleChanged: if (visible) opened = true

    Filters {
        id: filters
        onEdited: bar.edited()
    }

    function count(n: int): string { return Qt.locale().toString(n) }

    // A choice in a list facet: highlighted while the query has it. Each delegate binds `chosen` and `onClicked` to
    // its facet's calls on the Filters object (the enums' typed ones, or the string ones for extensions and ratings).
    component Chip: Button {
        // `revision` is what the delegates' bindings depend on: it changes whenever the query or the counts do.
        property bool chosen: false
        highlighted: chosen
        padding: 3
        leftPadding: 8
        rightPadding: 8
        // As wide as the text (Fusion's buttons are at least 80 px).
        implicitWidth: implicitContentWidth + leftPadding + rightPadding
        font.pixelSize: 12
    }

    // One end of a range.
    component Bound: TextField {
        id: field
        required property string facet
        required property string end
        property bool isDate: false
        readonly property bool bad: !filters.boundValid(facet, text)
        function reload() { text = filters.bound(facet, end) }
        Layout.preferredWidth: isDate ? 96 : 60
        placeholderText: isDate ? "YYYY-MM-DD" : end
        selectByMouse: true
        font.pixelSize: 12
        color: bad ? Theme.invalidText : palette.text
        onTextEdited: filters.setBound(facet, end, text)
        Component.onCompleted: reload()
        Connections {
            target: filters
            function onLoaded() { field.reload() }
        }
    }

    component Caption: Label {
        color: Theme.captionText
        font.pixelSize: 12
        Layout.leftMargin: 6
    }

    Loader {
        anchors.left: parent.left
        anchors.right: parent.right
        active: bar.opened
        sourceComponent: Component {
            Flow {
                spacing: 14

                RowLayout {
                    spacing: 4
                    Caption { text: "Type" }
                    Repeater {
                        model: bar.types
                        delegate: Chip {
                            required property var modelData
                            // `revision >= 0` is true for any loaded model: it makes this binding depend on `revision`,
                            // so it re-evaluates after a change. The model's roles replace these guards at the QML
                            // step (docs/REVIEW.md, R-63).
                            chosen: filters.revision >= 0 && filters.hasMediaType(modelData)
                            text: filters.mediaTypeLabel(modelData) + "  "
                                + bar.count(filters.revision >= 0 ? filters.mediaTypeCount(modelData) : 0)
                            onClicked: filters.toggleMediaType(modelData)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Rating" }
                    Repeater {
                        model: 6
                        delegate: Chip {
                            required property int index
                            chosen: filters.revision >= 0 && filters.has("ratings", "" + index)
                            text: Ratings.symbol(index) + (index === 0 ? " Unrated" : "")
                            onClicked: filters.toggle("ratings", "" + index)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Extension" }
                    Repeater {
                        model: filters.extensions
                        delegate: Chip {
                            required property string modelData
                            chosen: filters.revision >= 0 && filters.has("exts", modelData)
                            text: filters.label("exts", modelData) + "  "
                                + bar.count(filters.revision >= 0 ? filters.extCount(modelData) : 0)
                            onClicked: filters.toggle("exts", modelData)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Shape" }
                    Repeater {
                        model: bar.orientations
                        delegate: Chip {
                            required property var modelData
                            chosen: filters.revision >= 0 && filters.hasShape(modelData)
                            text: filters.shapeLabel(modelData)
                            onClicked: filters.toggleShape(modelData)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Size (MB)" }
                    Bound { facet: "size_mb"; end: "min" }
                    Bound { facet: "size_mb"; end: "max" }
                    Caption { text: "Megapixels" }
                    Bound { facet: "megapixels"; end: "min" }
                    Bound { facet: "megapixels"; end: "max" }
                    Caption { text: "Duration (s)" }
                    Bound { facet: "duration_s"; end: "min" }
                    Bound { facet: "duration_s"; end: "max" }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Taken" }
                    Bound { facet: "taken"; end: "from"; isDate: true }
                    Bound { facet: "taken"; end: "to"; isDate: true }
                    Caption { text: "Modified" }
                    Bound { facet: "modified"; end: "from"; isDate: true }
                    Bound { facet: "modified"; end: "to"; isDate: true }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Duplicates" }
                    ComboBox {
                        model: bar.duplicateChoices
                        currentIndex: filters.duplicates < 0 ? 0 : filters.duplicates > 0 ? 1 : 2
                        onActivated: (i) => filters.duplicates = i === 0 ? -1 : i === 1 ? 1 : 0
                        font.pixelSize: 12
                        Layout.preferredWidth: 140
                    }
                    Caption { text: "Import" }
                    Repeater {
                        model: bar.statuses
                        delegate: Chip {
                            required property var modelData
                            chosen: filters.revision >= 0 && filters.hasStatus(modelData)
                            text: filters.statusLabel(modelData)
                            onClicked: filters.toggleStatus(modelData)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Borders" }
                    Repeater {
                        model: bar.borderValues
                        delegate: Chip {
                            required property var modelData
                            chosen: filters.revision >= 0 && filters.hasBorder(modelData)
                            text: filters.borderLabel(modelData) + "  "
                                + bar.count(filters.revision >= 0 ? filters.borderValueCount(modelData) : 0)
                            onClicked: filters.toggleBorder(modelData)
                        }
                    }
                    Caption { text: "Review" }
                    ComboBox {
                        model: bar.reviewChoices
                        currentIndex: filters.borderMarked + 1
                        onActivated: (i) => filters.borderMarked = i - 1
                        font.pixelSize: 12
                        Layout.preferredWidth: 170
                    }
                    Button {
                        text: "Save cropped copies (" + bar.count(bar.croppable) + ")…"
                        enabled: bar.croppable > 0
                        font.pixelSize: 12
                        onClicked: bar.saveCropsRequested()
                        ToolTip.visible: hovered
                        ToolTip.delay: 500
                        ToolTip.text: "Copies of the listed files with their borders cut off, in a folder you choose "
                            + "(the originals stay as they are). JPEGs are cropped losslessly; their left and top "
                            + "cuts move inward to the next 8 or 16 px block."
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Name" }
                    TextField {
                        id: nameField
                        placeholderText: "contains…"
                        selectByMouse: true
                        font.pixelSize: 12
                        Layout.preferredWidth: 160
                        // Not bound to the query's name, which is trimmed: a space typed between two words
                        // would vanish.
                        function reload() { text = filters.name }
                        onTextEdited: filters.name = text
                        Component.onCompleted: reload()
                        Connections {
                            target: filters
                            function onLoaded() { nameField.reload() }
                        }
                    }
                    Button {
                        text: "Clear"
                        enabled: filters.active > 0
                        font.pixelSize: 12
                        onClicked: filters.clear()
                    }
                }
            }
        }
    }
}
