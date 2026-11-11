// A ListView whose mouse-wheel scrolling keeps up with a fast wheel.
//
// Qt Quick's own wheel scrolling (QQuickFlickable::wheelEvent, Qt 6.6+) eases each notch over 300 ms, but a new
// notch restarts the ease from wherever the last one had got to and drops the rest of it: the faster the wheel
// spins, the less of each notch is travelled, so speed tops out near 23 notches' worth of pixels per second
// however fast it spins (QTBUG-129948, QTBUG-116388). Here a notch moves a destination instead, as in Qt Widgets
// (QAbstractSlider::scrollByDelta adds every notch in full), and the view eases toward it each frame on Qt's own
// curve: a lone notch moves exactly as before, a burst of notches travels all of them.
//
// The handler is part of the view, not a component put in one, so it reaches the view by its id: the QML compiler
// compiles that to C++, where it can't for a Flickable found at run time (its members aren't final).
import QtQuick

ListView {
    id: list

    // Pixels per wheel line; a notch (120 units of angleDelta) is the system's lines per notch of them. Qt Quick's own
    // is 24 (QQuickFlickable::wheelEvent).
    property real wheelLineHeight: 24

    // A wheel's scroll came to rest (the view's own `movementEnded` is a drag's or a flick's): what the user scrolled to,
    // for a view that follows its end (LogPanel).
    signal wheelScrolled()

    // contentY's range, as QQuickFlickable / QQuickItemView compute it (-minYExtent … -maxYExtent). A ListView's
    // contentHeight and originY are estimates that firm up as rows are created, so this is re-read every frame.
    function clampContentY(y: real): real {
        const top = list.originY - list.topMargin
        const bottom = Math.max(top, list.originY + list.contentHeight + list.bottomMargin - list.height)
        return Math.max(top, Math.min(bottom, y))
    }

    // One wheel event, by its vertical fields (QWheelEvent's angleDelta.y, pixelDelta.y and phase): the handler below
    // calls this. It is a function, not only the handler's body, so the tests can drive it: QtTest's mouseWheel does not
    // reach a WheelHandler (a MouseArea's onWheel does), so the routing is checked by the app, not the tests. Typed
    // scalars, not the event object: the QML compiler needs the types to compile this to C++.
    function wheelBy(angleY: real, pixelY: real, phase: int) {
        if (!frames.running)
            scroller.destination = list.contentY
        // Pixel deltas from the platform (touchpads, on some systems): follow them as they come, which is what
        // they are for. Qt takes this path under the same conditions, except that it cannot see the event's
        // source here.
        if (pixelY !== 0 && phase !== Qt.NoScrollPhase) {
            frames.stop()
            scroller.moveTo(clampContentY(scroller.destination - pixelY))
            list.wheelScrolled()
            return
        }
        scroller.destination = clampContentY(
            scroller.destination - angleY / 120 * Application.styleHints.wheelScrollLines * wheelLineHeight)
        if (!frames.running) {
            scroller.written = contentY
            frames.start()
        }
    }

    // A handler declared in a Flickable is moved onto its contentItem (QQuickFlickablePrivate::addPointerHandler),
    // which hit-tests as the whole view (QQuickFlickableContentItem::contains) and is reached before the Flickable's
    // own wheelEvent().
    WheelHandler {
        id: scroller

        // Where the view is heading, and the contentY last written, to notice anything else moving it (a scrollbar
        // drag, a key, positionViewAtIndex), which ends the scroll. Qt itself moves it by less than a pixel: a
        // Flickable that is not "moving" (and writing contentY is not) snaps to a whole pixel whenever its
        // contentHeight changes (QQuickFlickable::setContentHeight -> QQuickFlickablePrivate::fixup), which a
        // ListView's does as it creates rows.
        property real destination: 0
        property real written: 0
        // Qt eases a notch with OutExpo over 300 ms: 1 - 2^(-10 t / 0.3 s), i.e. an exponential approach with this
        // time constant. Chasing the destination with it gives a lone notch the same motion.
        readonly property real tau: 0.3 / (10 * Math.LN2)

        function moveTo(y: real) {
            written = y
            list.contentY = y  // also stops a flick of the Flickable's own (setContentY resets its timeline)
        }

        // Plain wheel only: Ctrl+wheel and the like stay with other handlers (modifiers must match exactly).
        acceptedModifiers: Qt.NoModifier
        orientation: Qt.Vertical

        onWheel: (ev) => list.wheelBy(ev.angleDelta.y, ev.pixelDelta.y, ev.phase)
    }

    FrameAnimation {
        id: frames
        onTriggered: {
            if (Math.abs(list.contentY - scroller.written) >= 1) {
                stop()
                return
            }
            scroller.destination = list.clampContentY(scroller.destination)
            const rest = scroller.destination - list.contentY
            const done = Math.abs(rest) < 0.5
            scroller.moveTo(done ? scroller.destination : list.contentY + rest * (1 - Math.exp(-frameTime / scroller.tau)))
            if (done) {
                stop()
                list.wheelScrolled()
            }
        }
    }
}
