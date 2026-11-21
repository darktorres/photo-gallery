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
    Words {
        id: words
    }

    // How many facets filter; files in the folder, unfiltered, and those the query keeps (countFacets()).
    readonly property int active: filters.active
    readonly property int total: filters.total
    readonly property int matching: filters.matching
    // What the query keeps, in one line.
    readonly property string description: filters.description

    // The user changed the query.
    signal edited
    // Listed files with borders to crop off (GalleryModel.croppable), and the user asking to save them cropped.
    property int croppable: 0
    signal saveCropsRequested
    // Border detection is on: the borders controls act only then. Used while it is off, they say so (bordersBlocked),
    // and the crop and cut buttons with nothing to cut say so too (nothingToCrop).
    property bool bordersOn: true
    signal bordersBlocked
    signal nothingToCrop
    // The user asking to cut the borders off the listed originals (the window asks first: CutBordersDialog).
    signal cutBordersRequested

    // Count a merged folder's files (`""` = all folders), with or without its subfolders, for the counts.
    function countFacets() {
        filters.countFacets();
    }
    // The listing's query changed outside the bar (a saved view was applied): the fields show it again.
    function reloadQuery() {
        filters.reload();
    }

    // The bounded fields, by index: the order of pg_client::input::Field::ALL (pinned by its test). Each Bound names one
    // of these; a list facet's index above is a different namespace, so these are not facet indices.
    readonly property int fieldSize: 0
    readonly property int fieldMegapixels: 1
    readonly property int fieldDuration: 2
    readonly property int fieldTaken: 3
    readonly property int fieldModified: 4
    // The duplicates and review boxes, in their order (Filters.duplicatesIndex, borderMarkedIndex).
    readonly property list<string> duplicateChoices: [words.text("ui-any"), words.text("ui-has-duplicates"), words.text("ui-no-duplicates")]
    readonly property list<string> reviewChoices: [words.text("ui-any"), words.text("ui-not-marked"), words.text("ui-marked-not-a-border")]

    padding: 6

    // Shown once: the controls are made then (the window starts with the bar closed), and kept.
    property bool opened: false
    onVisibleChanged: if (visible)
        opened = true

    Filters {
        id: filters
        onEdited: bar.edited()
    }

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
        function reload() {
            text = filters.bound(facet, end);
        }
        Layout.preferredWidth: isDate ? 96 : 60
        placeholderText: isDate ? words.text("ui-date-format") : (end === 0 ? words.text("ui-min") : words.text("ui-max"))
        selectByMouse: true
        font.pixelSize: 12
        color: bad ? Theme.invalidText : palette.text
        onTextEdited: filters.setBound(facet, end, text)
        Component.onCompleted: reload()
        Connections {
            target: filters
            function onLoaded() {
                field.reload();
            }
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
                // They read the query and the counts themselves; each change to either asks for the rows again. A list
                // is named by the bridge's enum (FacetChoices.Types and so on); Filters.toggle takes the same names.
                FacetChoices {
                    id: typeChoices
                    facet: FacetChoices.Types
                }
                FacetChoices {
                    id: ratingChoices
                    facet: FacetChoices.Ratings
                }
                FacetChoices {
                    id: extChoices
                    facet: FacetChoices.Exts
                }
                FacetChoices {
                    id: shapeChoices
                    facet: FacetChoices.Shapes
                }
                FacetChoices {
                    id: statusChoices
                    facet: FacetChoices.Status
                }
                FacetChoices {
                    id: borderChoices
                    facet: FacetChoices.Borders
                }

                function refreshChoices() {
                    reviewBox.currentIndex = filters.borderMarkedIndex;
                    typeChoices.refresh();
                    ratingChoices.refresh();
                    extChoices.refresh();
                    shapeChoices.refresh();
                    statusChoices.refresh();
                    borderChoices.refresh();
                }

                Connections {
                    target: filters
                    function onEdited() {
                        choiceRows.refreshChoices();
                    }
                    function onLoaded() {
                        choiceRows.refreshChoices();
                    }
                    function onCounted() {
                        choiceRows.refreshChoices();
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-type")
                    }
                    Repeater {
                        model: typeChoices
                        delegate: Chip {
                            text: label + " · " + words.count(count)
                            onClicked: filters.toggle(Filters.Types, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-rating")
                    }
                    Repeater {
                        model: ratingChoices
                        delegate: Chip {
                            required property int index
                            text: Ratings.symbol(index) + (index === 0 ? " " + words.text("ui-unrated") : "")
                            onClicked: filters.toggle(Filters.Ratings, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-extension")
                    }
                    Repeater {
                        model: extChoices
                        delegate: Chip {
                            text: label + " · " + words.count(count)
                            onClicked: filters.toggle(Filters.Exts, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-shape")
                    }
                    Repeater {
                        model: shapeChoices
                        delegate: Chip {
                            text: label
                            onClicked: filters.toggle(Filters.Shapes, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-size-mb")
                    }
                    Bound {
                        facet: bar.fieldSize
                        end: 0
                    }
                    Bound {
                        facet: bar.fieldSize
                        end: 1
                    }
                    Caption {
                        text: words.text("ui-megapixels")
                    }
                    Bound {
                        facet: bar.fieldMegapixels
                        end: 0
                    }
                    Bound {
                        facet: bar.fieldMegapixels
                        end: 1
                    }
                    Caption {
                        text: words.text("ui-duration-s")
                    }
                    Bound {
                        facet: bar.fieldDuration
                        end: 0
                    }
                    Bound {
                        facet: bar.fieldDuration
                        end: 1
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-taken")
                    }
                    Bound {
                        facet: bar.fieldTaken
                        end: 0
                        isDate: true
                    }
                    Bound {
                        facet: bar.fieldTaken
                        end: 1
                        isDate: true
                    }
                    Caption {
                        text: words.text("ui-modified")
                    }
                    Bound {
                        facet: bar.fieldModified
                        end: 0
                        isDate: true
                    }
                    Bound {
                        facet: bar.fieldModified
                        end: 1
                        isDate: true
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-duplicates")
                    }
                    ComboBox {
                        model: bar.duplicateChoices
                        currentIndex: filters.duplicatesIndex
                        onActivated: i => filters.chooseDuplicates(i)
                        font.pixelSize: 12
                        Layout.preferredWidth: 140
                    }
                    Caption {
                        text: words.text("ui-import")
                    }
                    Repeater {
                        model: statusChoices
                        delegate: Chip {
                            text: label
                            onClicked: filters.toggle(Filters.Status, key)
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-borders")
                    }
                    Repeater {
                        model: borderChoices
                        delegate: Chip {
                            text: label + " · " + words.count(count)
                            onClicked: bar.bordersOn ? filters.toggle(Filters.Borders, key) : bar.bordersBlocked()
                        }
                    }
                    Caption {
                        text: words.text("ui-review")
                    }
                    ComboBox {
                        id: reviewBox
                        model: bar.reviewChoices
                        currentIndex: filters.borderMarkedIndex
                        onActivated: i => {
                            // Index 0 is "any", which needs no borders. Any other choice is refused while they are off:
                            // the box shows the review in force again (refreshChoices keeps it in step with the query).
                            if (i === 0 || bar.bordersOn) {
                                filters.chooseBorderMarked(i);
                            } else {
                                bar.bordersBlocked();
                                currentIndex = filters.borderMarkedIndex;
                            }
                        }
                        font.pixelSize: 12
                        Layout.preferredWidth: 170
                    }
                    Button {
                        text: words.phrase("ui-save-crops", 0, words.count(bar.croppable), "")
                        font.pixelSize: 12
                        onClicked: {
                            if (!bar.bordersOn)
                                bar.bordersBlocked();
                            else if (bar.croppable > 0)
                                bar.saveCropsRequested();
                            else
                                bar.nothingToCrop();
                        }
                        ToolTip.visible: hovered
                        ToolTip.delay: 500
                        ToolTip.text: words.text("ui-crop-tooltip")
                    }
                    Button {
                        text: words.text("ui-cut-originals")
                        font.pixelSize: 12
                        onClicked: {
                            if (!bar.bordersOn)
                                bar.bordersBlocked();
                            else if (bar.croppable > 0)
                                bar.cutBordersRequested();
                            else
                                bar.nothingToCrop();
                        }
                        ToolTip.visible: hovered
                        ToolTip.delay: 500
                        ToolTip.text: words.text("ui-cut-tooltip")
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-name")
                    }
                    TextField {
                        id: nameField
                        placeholderText: words.text("ui-contains")
                        selectByMouse: true
                        font.pixelSize: 12
                        Layout.preferredWidth: 160
                        // Not bound to the query's name, which is trimmed: a space typed between two words
                        // would vanish.
                        function reload() {
                            text = filters.name;
                        }
                        onTextEdited: filters.editName(text)
                        Component.onCompleted: reload()
                        Connections {
                            target: filters
                            function onLoaded() {
                                nameField.reload();
                            }
                        }
                    }
                    Button {
                        text: words.text("ui-clear")
                        enabled: filters.active > 0
                        font.pixelSize: 12
                        onClicked: filters.clear()
                    }
                }
            }
        }
    }
}
