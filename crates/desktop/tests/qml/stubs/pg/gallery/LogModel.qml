// A stub of the log view's model (crates/desktop/src/log_model.rs): the records' roles, as a ListModel has them (`seq`
// is the row's own number), with `ready` and `start`. A test fills it as tst_log_panel does.
import QtQuick

ListModel {
    property bool ready: false

    function start() {
    }
}
