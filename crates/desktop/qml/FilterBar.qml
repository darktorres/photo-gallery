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

    // How many facets filter; files in the folder, unfiltered, and those the query keeps (countFacets()).
    readonly property int active: filters.active
    readonly property int total: filters.total
    readonly property int matching: filters.matching
    // What the query keeps, in one line.
    readonly property string description: filters.description

    // The user changed the query.
    signal edited()
    // Listed files with borders to crop off (GalleryModel.croppable), and the user asking to save them cropped.
    property int croppable: 0
    signal saveCropsRequested()

    // Count a merged folder's files (`""` = all folders), with or without its subfolders, for the counts.
    function countFacets(folder: string, recursive: bool) { filters.countFacets(folder, recursive) }
    // The listing's query changed outside the bar (a saved view was applied): the fields show it again.
    function reloadQuery() { filters.reload() }

    // The list facets, by index: the order of pg_client::facets::Facet (pinned by tests/facets.rs). A FacetChoices row
    // set is made for each, in the flow below.
    readonly property int facetTypes: 0
    readonly property int facetOrientations: 1
    readonly property int facetStatus: 2
    readonly property int facetBorders: 3
    readonly property int facetRatings: 4
    readonly property int facetExts: 5
    // The duplicates and review boxes, in their order (Filters.duplicatesIndex, borderMarkedIndex).
    readonly property list<string> duplicateChoices: ["Any", "Has duplicates", "No duplicates"]
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

    // A choice in a list facet, one row of a FacetChoices: highlighted while the query has it (`chosen`). Each
    // delegate's `onClicked` toggles its row's key in its facet (`Filters.toggle`).
    component Chip: Button {
        required property string key
        required property string label
        required property int count
        required property bool chosen
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
        // The field (input.rs Field::ALL) and the end (Edge: 0 the low end, 1 the high end), as the core numbers them.
        required property int facet
        required property int end
        property bool isDate: false
        readonly property bool bad: !filters.boundValid(facet, text)
        function reload() { text = filters.bound(facet, end) }
        Layout.preferredWidth: isDate ? 96 : 60
        placeholderText: isDate ? "YYYY-MM-DD" : (end === 0 ? "min" : "max")
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
                id: choiceRows
                spacing: 14

                // The rows of each list facet (facet_choices.rs): a chip reads its row's key, label, count and chosen.
                // They read the query and the counts themselves; each change to either asks for the rows again.
                FacetChoices { id: typeChoices; facet: bar.facetTypes }
                FacetChoices { id: ratingChoices; facet: bar.facetRatings }
                FacetChoices { id: extChoices; facet: bar.facetExts }
                FacetChoices { id: shapeChoices; facet: bar.facetOrientations }
                FacetChoices { id: statusChoices; facet: bar.facetStatus }
                FacetChoices { id: borderChoices; facet: bar.facetBorders }

                function refreshChoices() {
                    typeChoices.refresh()
                    ratingChoices.refresh()
                    extChoices.refresh()
                    shapeChoices.refresh()
                    statusChoices.refresh()
                    borderChoices.refresh()
                }

                Connections {
                    target: filters
                    function onEdited() { choiceRows.refreshChoices() }
                    function onLoaded() { choiceRows.refreshChoices() }
                    function onCounted() { choiceRows.refreshChoices() }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Type" }
                    Repeater {
                        model: typeChoices
                        delegate: Chip {
                            text: label + "  " + bar.count(count)
                            onClicked: filters.toggle(bar.facetTypes, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Rating" }
                    Repeater {
                        model: ratingChoices
                        delegate: Chip {
                            required property int index
                            text: Ratings.symbol(index) + (index === 0 ? " Unrated" : "")
                            onClicked: filters.toggle(bar.facetRatings, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Extension" }
                    Repeater {
                        model: extChoices
                        delegate: Chip {
                            text: label + "  " + bar.count(count)
                            onClicked: filters.toggle(bar.facetExts, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Shape" }
                    Repeater {
                        model: shapeChoices
                        delegate: Chip {
                            text: label
                            onClicked: filters.toggle(bar.facetOrientations, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Size (MB)" }
                    Bound { facet: 0; end: 0 }
                    Bound { facet: 0; end: 1 }
                    Caption { text: "Megapixels" }
                    Bound { facet: 1; end: 0 }
                    Bound { facet: 1; end: 1 }
                    Caption { text: "Duration (s)" }
                    Bound { facet: 2; end: 0 }
                    Bound { facet: 2; end: 1 }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Taken" }
                    Bound { facet: 3; end: 0; isDate: true }
                    Bound { facet: 3; end: 1; isDate: true }
                    Caption { text: "Modified" }
                    Bound { facet: 4; end: 0; isDate: true }
                    Bound { facet: 4; end: 1; isDate: true }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Duplicates" }
                    ComboBox {
                        model: bar.duplicateChoices
                        currentIndex: filters.duplicatesIndex
                        onActivated: (i) => filters.setDuplicatesIndex(i)
                        font.pixelSize: 12
                        Layout.preferredWidth: 140
                    }
                    Caption { text: "Import" }
                    Repeater {
                        model: statusChoices
                        delegate: Chip {
                            text: label
                            onClicked: filters.toggle(bar.facetStatus, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption { text: "Borders" }
                    Repeater {
                        model: borderChoices
                        delegate: Chip {
                            text: label + "  " + bar.count(count)
                            onClicked: filters.toggle(bar.facetBorders, key)
                        }
                    }
                    Caption { text: "Review" }
                    ComboBox {
                        model: bar.reviewChoices
                        currentIndex: filters.borderMarkedIndex
                        onActivated: (i) => filters.setBorderMarkedIndex(i)
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
