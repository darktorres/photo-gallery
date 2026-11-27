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

    // How many facets filter, and how many files the folder has unfiltered (countFacets()).
    readonly property int active: filters.active
    readonly property int total: filters.total
    // The folder's counts for what is shown are not in yet (Filters.countsPending): the total and the choices' counts
    // are shown as pending, not as the 0 or the last folder's numbers they hold meanwhile.
    readonly property bool countsPending: filters.countsPending
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
    // Duplicate detection is on: the duplicates choices that need it act only then, and are refused (duplicatesBlocked) while
    // it is off, as the review choices are while borders are.
    property bool duplicatesOn: true
    signal duplicatesBlocked
    // A cut is under way (the library is busy): the cut button is off until it is done.
    property bool busy: false
    // The keys go back to the grid: a control that the query changed under (a chip, Clear) hands them over.
    signal focusGridRequested

    // The bar opened or closed (the window reports it). The model counts the choices while the bar is open.
    function setBarOpen(open: bool) {
        filters.setBarOpen(open);
    }
    // The choices are shown again (the listing changed): the model counts them when they are needed.
    function countIfNeeded() {
        filters.countIfNeeded();
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
    readonly property list<string> duplicateChoices: [words.text("ui-any"), words.text("ui-with-duplicates"), words.text("ui-without-duplicates")]
    readonly property list<string> reviewChoices: [words.text("ui-any"), words.text("ui-not-marked"), words.text("ui-marked-not-a-border")]

    padding: 6

    // Shown once: the controls are made then (the window starts with the bar closed), and kept.
    property bool opened: false
    onVisibleChanged: if (visible)
        opened = true
    // Made already visible (the start's --filters): no visibleChanged comes, so the controls are asked for here.
    Component.onCompleted: if (visible)
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
        // The count is not in yet (FacetChoices role "pending"): the chip shows a mark for it, dimmed, not a 0.
        required property bool pending
        highlighted: chosen
        opacity: pending ? 0.5 : 1
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
                            text: label + " · " + (pending ? words.text("ui-count-pending") : words.count(count))
                            onClicked: {
                                filters.toggle(Filters.Types, key);
                                bar.focusGridRequested();
                            }
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
                            // ☆ and its words (unrated) in the symbol font; the other ratings are emoji alone.
                            font.family: index === 0 ? Theme.symbolFamily : Theme.emojiFamily
                            onClicked: {
                                filters.toggle(Filters.Ratings, key);
                                bar.focusGridRequested();
                            }
                        }
                    }
                }

                RowLayout {
                    spacing: 4
                    Caption {
                        text: words.text("ui-extension")
                    }
                    // The folder's extensions are not counted yet: there are no choices to show, so the row says so.
                    Label {
                        visible: extRepeater.count === 0 && filters.countsPending
                        text: words.text("ui-counting")
                        color: Theme.captionText
                        font.pixelSize: 12
                    }
                    Repeater {
                        id: extRepeater
                        model: extChoices
                        delegate: Chip {
                            text: label + " · " + (pending ? words.text("ui-count-pending") : words.count(count))
                            onClicked: {
                                filters.toggle(Filters.Exts, key);
                                bar.focusGridRequested();
                            }
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
                            onClicked: {
                                filters.toggle(Filters.Shapes, key);
                                bar.focusGridRequested();
                            }
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
                        // Tabbed to, not clicked to: a pick leaves the focus where it was, and the arrow keys keep the
                        // box's own steps (not the grid's moves).
                        focusPolicy: Qt.TabFocus
                        // "Any" needs no duplicates; the others are refused while detection is off (the box shows the
                        // choice in force again).
                        onActivated: i => {
                            if (i === 0 || bar.duplicatesOn) {
                                filters.chooseDuplicates(i);
                            } else {
                                bar.duplicatesBlocked();
                                currentIndex = filters.duplicatesIndex;
                            }
                        }
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
                            onClicked: {
                                filters.toggle(Filters.Status, key);
                                bar.focusGridRequested();
                            }
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
                            text: label + " · " + (pending ? words.text("ui-count-pending") : words.count(count))
                            onClicked: {
                            if (bar.bordersOn)
                                filters.toggle(Filters.Borders, key);
                            else
                                bar.bordersBlocked();
                            bar.focusGridRequested();
                        }
                        }
                    }
                    Caption {
                        text: words.text("ui-review")
                    }
                    ComboBox {
                        id: reviewBox
                        model: bar.reviewChoices
                        currentIndex: filters.borderMarkedIndex
                        // Tabbed to, not clicked to (as the duplicates box): the arrow keys step the box, not the grid.
                        focusPolicy: Qt.TabFocus
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
                        // Off while a job runs: a second cut would queue the same originals again.
                        enabled: !bar.busy
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
                        // Clearing turns this button off, which takes the focus it has: the keys go back to the grid.
                        onClicked: {
                            filters.clear();
                            bar.focusGridRequested();
                        }
                        ToolTip.visible: hovered
                        ToolTip.delay: 500
                        ToolTip.text: words.text("ui-clear-filters-tooltip")
                    }
                }
            }
        }
    }
}
