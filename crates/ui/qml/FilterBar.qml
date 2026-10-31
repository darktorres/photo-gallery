// The filter bar, under the top bar: one control per facet of a query::Query (crates/core/src/query.rs).
// Facets combine with AND, the choices inside one with OR, and an empty facet doesn't filter. Each choice shows
// how many files it would show given the other facets (gallery.facets). Edits change `query` and fire edited();
// the window applies them after a short pause.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import pg.gallery

Pane {
    id: bar
    // query::Query as a JS object: only the facets that filter are present.
    property var query: ({})
    // gallery.facets(), parsed: total, matching, types (type → count), exts ([extension, count], most first).
    property var facets: ({ total: 0, matching: 0, types: {}, exts: [] })
    readonly property int active: Object.keys(query).length
    signal edited()
    // The text fields reload from `query` (load()).
    signal loaded()
    padding: 6

    // Replace the whole query (a saved view, the session, Clear).
    function load(q) {
        query = q || {}
        loaded()
    }

    function set(key, value) {
        const q = Object.assign({}, query)
        const empty = value === undefined || value === null || value === ""
            || (Array.isArray(value) && value.length === 0)
            || (typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0)
        if (empty)
            delete q[key]
        else
            q[key] = value
        query = q
        edited()
    }

    function has(key, value) { return (query[key] || []).indexOf(value) >= 0 }

    function toggle(key, value) {
        const list = (query[key] || []).slice()
        const i = list.indexOf(value)
        if (i >= 0)
            list.splice(i, 1)
        else
            list.push(value)
        set(key, list)
    }

    // One end of a range: `end` is min/max (numbers) or from/to (dates); unparsable text leaves it open.
    function setBound(key, end, value) {
        const r = Object.assign({}, query[key] || {})
        if (value === undefined)
            delete r[end]
        else
            r[end] = value
        set(key, r)
    }

    function number(text) {
        const v = Number(text.trim().replace(",", "."))
        return text.trim().length && !isNaN(v) ? v : undefined
    }
    function date(text) {
        const t = text.trim()
        return /^\d{4}-\d{2}-\d{2}$/.test(t) && !isNaN(Date.parse(t)) ? t : undefined
    }
    function count(n) { return (n || 0).toLocaleString(Qt.locale(), "f", 0) }

    readonly property var typeNames: ({ image: "Image", animated: "Animated", video: "Video" })
    readonly property var statusNames: ({ imported: "Imported", pending: "Not imported yet", failed: "Unreadable" })

    // One line saying what the query keeps, for the save-view dialog.
    function describe() {
        const parts = []
        const range = (r, unit) => (r.min !== undefined ? r.min : "") + "–" + (r.max !== undefined ? r.max : "") + unit
        const dates = (r) => (r.from || "") + " – " + (r.to || "")
        const q = query
        if (q.types) parts.push(q.types.map((t) => typeNames[t]).join(", "))
        if (q.exts) parts.push(q.exts.join(", "))
        if (q.ratings) parts.push(q.ratings.map((n) => n === 0 ? "unrated" : Ratings.symbol(n)).join(" "))
        if (q.size_mb) parts.push(range(q.size_mb, " MB"))
        if (q.megapixels) parts.push(range(q.megapixels, " MP"))
        if (q.orientations) parts.push(q.orientations.join(", "))
        if (q.taken) parts.push("taken " + dates(q.taken))
        if (q.modified) parts.push("modified " + dates(q.modified))
        if (q.duration_s) parts.push(range(q.duration_s, " s"))
        if (q.duplicates !== undefined) parts.push(q.duplicates ? "with duplicates" : "without duplicates")
        if (q.status) parts.push(q.status.map((s) => statusNames[s].toLowerCase()).join(", "))
        if (q.name) parts.push("name has “" + q.name + "”")
        return parts.length ? parts.join(" · ") : "all files"
    }

    // A choice in a facet: highlighted while chosen.
    component Chip: Button {
        property string key
        property var value
        highlighted: bar.has(key, value)
        padding: 3
        leftPadding: 8
        rightPadding: 8
        // As wide as the text (Fusion's buttons are at least 80 px).
        implicitWidth: implicitContentWidth + leftPadding + rightPadding
        font.pixelSize: 12
        onClicked: bar.toggle(key, value)
    }

    // One end of a range.
    component Bound: TextField {
        id: field
        property string key
        property string end
        property bool isDate: false
        readonly property bool bad: text.trim().length > 0 && parse(text) === undefined
        function parse(t) { return isDate ? bar.date(t) : bar.number(t) }
        function reload() {
            const r = bar.query[key]
            text = r && r[end] !== undefined ? String(r[end]) : ""
        }
        Layout.preferredWidth: isDate ? 96 : 60
        placeholderText: isDate ? "YYYY-MM-DD" : (end === "min" ? "min" : "max")
        selectByMouse: true
        font.pixelSize: 12
        color: bad ? "#e06060" : palette.text
        onTextEdited: bar.setBound(key, end, parse(text))
        Connections {
            target: bar
            function onLoaded() { field.reload() }
        }
    }

    component Caption: Label {
        color: "#999999"
        font.pixelSize: 12
        Layout.leftMargin: 6
    }

    Flow {
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: 14

        RowLayout {
            spacing: 4
            Caption { text: "Type" }
            Repeater {
                model: ["image", "animated", "video"]
                delegate: Chip {
                    required property string modelData
                    key: "types"
                    value: modelData
                    text: bar.typeNames[modelData] + "  " + bar.count(bar.facets.types[modelData])
                }
            }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Rating" }
            Repeater {
                model: [0, 1, 2, 3, 4, 5]
                delegate: Chip {
                    required property int modelData
                    key: "ratings"
                    value: modelData
                    text: Ratings.symbol(modelData) + (modelData === 0 ? " Unrated" : "")
                }
            }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Extension" }
            Repeater {
                // The extensions in this folder, plus any chosen that aren't (so they can be unchosen).
                model: {
                    const exts = (bar.facets.exts || []).slice()
                    for (const e of bar.query.exts || [])
                        if (!exts.some((x) => x[0] === e))
                            exts.push([e, 0])
                    return exts
                }
                delegate: Chip {
                    required property var modelData
                    key: "exts"
                    value: modelData[0]
                    text: (modelData[0].length ? modelData[0] : "(none)") + "  " + bar.count(modelData[1])
                }
            }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Shape" }
            Repeater {
                model: [["landscape", "Landscape"], ["portrait", "Portrait"], ["square", "Square"]]
                delegate: Chip {
                    required property var modelData
                    key: "orientations"
                    value: modelData[0]
                    text: modelData[1]
                }
            }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Size (MB)" }
            Bound { key: "size_mb"; end: "min" }
            Bound { key: "size_mb"; end: "max" }
            Caption { text: "Megapixels" }
            Bound { key: "megapixels"; end: "min" }
            Bound { key: "megapixels"; end: "max" }
            Caption { text: "Duration (s)" }
            Bound { key: "duration_s"; end: "min" }
            Bound { key: "duration_s"; end: "max" }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Taken" }
            Bound { key: "taken"; end: "from"; isDate: true }
            Bound { key: "taken"; end: "to"; isDate: true }
            Caption { text: "Modified" }
            Bound { key: "modified"; end: "from"; isDate: true }
            Bound { key: "modified"; end: "to"; isDate: true }
        }

        RowLayout {
            spacing: 4
            Caption { text: "Duplicates" }
            ComboBox {
                id: dupBox
                textRole: "text"
                valueRole: "value"
                model: [
                    { value: "any", text: "Any" },
                    { value: "yes", text: "Has duplicates" },
                    { value: "no", text: "No duplicates" },
                ]
                currentIndex: bar.query.duplicates === undefined ? 0 : bar.query.duplicates ? 1 : 2
                onActivated: bar.set("duplicates", currentValue === "any" ? undefined : currentValue === "yes")
                font.pixelSize: 12
                Layout.preferredWidth: 140
            }
            Caption { text: "Import" }
            Repeater {
                model: ["imported", "pending", "failed"]
                delegate: Chip {
                    required property string modelData
                    key: "status"
                    value: modelData
                    text: bar.statusNames[modelData]
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
                onTextEdited: bar.set("name", text.trim())
                Connections {
                    target: bar
                    function onLoaded() { nameField.text = bar.query.name || "" }
                }
            }
            Button {
                text: "Clear"
                enabled: bar.active > 0
                font.pixelSize: 12
                onClicked: {
                    bar.load({})
                    bar.edited()
                }
            }
        }
    }
}
