// A stub of the inbox's model (crates/desktop/src/notices.rs): its roles are a ListModel's, and the properties and calls
// the QML uses are here. The calls do nothing; a test sets the properties it needs.
import QtQuick

ListModel {
    property int unread: 0
    property int passes: 0
    property string toast: ""
    property int toastLevel: 0

    function start() {
    }
    function setViewing(open, active) {
    }
    function dismiss(row) {
    }
    function dismissAll() {
    }
    function copy(row) {
    }
    function dismissToast() {
    }
    function poke() {
    }
    function rowOfKey(key) {
        return -1
    }
    function passKey(pass) {
        return ""
    }
    function passOfCode(code) {
        return -1
    }
    function retry(row) {
    }
}
