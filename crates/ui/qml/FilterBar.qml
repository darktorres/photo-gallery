// The filter bar, under the top bar: one control per facet of the query (Filters, src/filters.rs, editing a
// query::Query). Facets combine with AND, the choices inside one with OR, and an empty facet doesn't filter. Each
// type and extension shows how many files choosing it would show given the other facets (countFacets()). Edits go
// straight to the query, and edited() fires; the window applies them after a short pause.
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

    // Count a merged folder's files (`""` = all folders), with or without its subfolders, for the counts.
    function countFacets(folder: string, recursive: bool) { filters.countFacets(folder, recursive) }

    readonly property list<string> types: ["image", "animated", "video"]
    readonly property list<string> orientations: ["landscape", "portrait", "square"]
    readonly property list<string> statuses: ["imported", "pending", "failed"]
    // Filters.duplicates: -1 either way, 1 with a duplicate, 0 without; in the duplicates box's order.
    readonly property list<string> duplicateChoices: ["Any", "Has duplicates", "No duplicates"]

    padding: 6

    // Shown once: the controls are made then (the window starts with the bar closed), and kept.
    property bool opened: false
    onVisibleChanged: if (visible) opened = true

    Filters {
        id: filters
        symbols: Ratings.symbols
        onEdited: bar.edited()
    }

    function count(n: int): string { return Qt.locale().toString(n) }

    // A choice in a facet: highlighted while chosen.
    component Chip: Button {
        required property string facet
        required property string value
        highlighted: filters.revision >= 0 && filters.has(facet, value)
        padding: 3
        leftPadding: 8
        rightPadding: 8
        // As wide as the text (Fusion's buttons are at least 80 px).
        implicitWidth: implicitContentWidth + leftPadding + rightPadding
        font.pixelSize: 12
        onClicked: filters.toggle(facet, value)
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
        color: bad ? "#e06060" : palette.text
        onTextEdited: filters.setBound(facet, end, text)
        Component.onCompleted: reload()
        Connections {
            target: filters
            function onLoaded() { field.reload() }
        }
    }

    component Caption: Label {
        color: "#999999"
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
                            required property string modelData
                            facet: "types"
                            value: modelData
                            text: filters.label(facet, modelData) + "  "
                                + bar.count(filters.revision >= 0 ? filters.typeCount(modelData) : 0)
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
                            facet: "ratings"
                            value: "" + index
                            text: Ratings.symbol(index) + (index === 0 ? " Unrated" : "")
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
                            facet: "exts"
                            value: modelData
                            text: filters.label(facet, modelData) + "  "
                                + bar.count(filters.revision >= 0 ? filters.extCount(modelData) : 0)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Shape" }
                    Repeater {
                        model: bar.orientations
                        delegate: Chip {
                            required property string modelData
                            facet: "orientations"
                            value: modelData
                            text: filters.label(facet, modelData)
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
                            required property string modelData
                            facet: "status"
                            value: modelData
                            text: filters.label(facet, modelData)
                        }
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
                        // Not bound to the query's name, which is trimmed: a space typed between two words would vanish.
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
