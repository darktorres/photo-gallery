// The mouse wheel's units, each written once. A notch is 120 units of angleDelta (Qt's QWheelEvent::DefaultDeltasPerStep);
// the handlers that turn a wheel into notches or scroll lines divide by it.
pragma Singleton
pragma ComponentBehavior: Bound
import QtQuick

QtObject {
    // The angleDelta of one wheel notch.
    readonly property real notch: 120
}
