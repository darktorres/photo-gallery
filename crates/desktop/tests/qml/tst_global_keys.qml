// The window's shortcuts (GlobalShortcuts.qml): each fires wherever the focus is, before the focused item's keys, so
// the grid's letter keys (which take a letter whatever its modifiers) and a text field never see it. Run by `just qmltest`.
import QtQuick
import QtQuick.Controls.Fusion
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "GlobalKeys"
    // Items created under a non-visible TestCase are not visible, so no key reaches them (see tst_wheellistview).
    visible: true
    when: windowShown
    width: 400
    height: 200

    GlobalShortcuts {
        id: shortcuts
    }
    SignalSpy { id: actionSpy; target: shortcuts; signalName: "activated" }

    // An item that takes every key it is sent, as the grid does; it counts the letters it gets (keyClick sends the
    // modifiers as keys of their own first).
    Item {
        id: keyed
        width: 100
        height: 50
        property int pressed: 0
        Keys.onPressed: (event) => {
            if (event.key >= Qt.Key_A && event.key <= Qt.Key_Z)
                pressed++
            event.accepted = true
        }
    }
    TextField {
        id: field
        y: 60
        width: 200
    }

    function test_a_shortcut_fires_before_the_focused_items_keys_data() {
        return [
            { tag: "inbox from the grid", key: Qt.Key_N, code: GlobalShortcuts.Inbox, target: "keyed" },
            { tag: "inbox from a text field", key: Qt.Key_N, code: GlobalShortcuts.Inbox, target: "field" },
            { tag: "log from the grid", key: Qt.Key_L, code: GlobalShortcuts.Log, target: "keyed" },
            { tag: "log from a text field", key: Qt.Key_L, code: GlobalShortcuts.Log, target: "field" },
            { tag: "dev mode from the grid", key: Qt.Key_D, code: GlobalShortcuts.DevMode, target: "keyed" },
            { tag: "dev mode from a text field", key: Qt.Key_D, code: GlobalShortcuts.DevMode, target: "field" },
        ]
    }

    function test_a_shortcut_fires_before_the_focused_items_keys(data) {
        const target = data.target === "keyed" ? keyed : field
        target.forceActiveFocus()
        verify(target.activeFocus)
        actionSpy.clear()
        keyed.pressed = 0
        field.text = ""
        keyClick(data.key, Qt.ControlModifier | Qt.ShiftModifier)
        compare(actionSpy.count, 1)
        compare(actionSpy.signalArguments[0][0], data.code)
        compare(keyed.pressed, 0)
        compare(field.text, "")
    }

    // A plain letter is the focused item's, not a shortcut.
    function test_a_plain_letter_is_the_items() {
        keyed.forceActiveFocus()
        actionSpy.clear()
        keyed.pressed = 0
        keyClick(Qt.Key_L)
        compare(actionSpy.count, 0)
        compare(keyed.pressed, 1)
    }
}
