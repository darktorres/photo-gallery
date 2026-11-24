// A Flickable whose mouse-wheel scrolling keeps up with a fast wheel: WheelListView for a view that is not a list of
// rows (the Yandex results' justified blocks, in a Flickable of their own).
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
pragma ComponentBehavior: Bound
import QtQuick

Flickable {
    id: flick

    // Pixels per wheel line; a notch (120 units of angleDelta) is the system's lines per notch of them. Qt Quick's own
    // is 24 (QQuickFlickable::wheelEvent).
    property real wheelLineHeight: 24

    // contentY's range, as QQuickFlickable computes it (-minYExtent … -maxYExtent). The content's height is read again
    // every frame, as the content may change under the view.
    function clampContentY(y: real): real {
        const top = flick.originY - flick.topMargin;
        const bottom = Math.max(top, flick.originY + flick.contentHeight + flick.bottomMargin - flick.height);
        return Math.max(top, Math.min(bottom, y));
    }

    // One wheel event, by its vertical fields (QWheelEvent's angleDelta.y, pixelDelta.y and phase): the handler below
    // calls this. It is a function, not only the handler's body, so the tests can drive it: QtTest's mouseWheel does not
    // reach a WheelHandler (a MouseArea's onWheel does), so the routing is checked by the app, not the tests. Typed
    // scalars, not the event object: the QML compiler needs the types to compile this to C++.
    function wheelBy(angleY: real, pixelY: real, phase: int) {
        if (!frames.running)
            scroller.destination = flick.contentY;
        // Pixel deltas from the platform (touchpads, on some systems): follow them as they come, which is what
        // they are for. Qt takes this path under the same conditions, except that it cannot see the event's
        // source here.
        if (pixelY !== 0 && phase !== Qt.NoScrollPhase) {
            frames.stop();
            scroller.moveTo(flick.clampContentY(scroller.destination - pixelY));
            return;
        }
        scroller.destination = flick.clampContentY(scroller.destination - angleY / Wheel.notch * Application.styleHints.wheelScrollLines * flick.wheelLineHeight);
        if (!frames.running) {
            scroller.written = flick.contentY;
            frames.start();
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
        // contentHeight changes (QQuickFlickable::setContentHeight -> QQuickFlickablePrivate::fixup).
        property real destination: 0
        property real written: 0
        // Qt eases a notch with OutExpo over 300 ms: 1 - 2^(-10 t / 0.3 s), i.e. an exponential approach with this
        // time constant. Chasing the destination with it gives a lone notch the same motion.
        readonly property real tau: 0.3 / (10 * Math.LN2)

        function moveTo(y: real) {
            written = y;
            flick.contentY = y;  // also stops a flick of the Flickable's own (setContentY resets its timeline)
        }

        // Plain wheel only: Ctrl+wheel and the like stay with other handlers (modifiers must match exactly).
        acceptedModifiers: Qt.NoModifier
        orientation: Qt.Vertical

        onWheel: ev => flick.wheelBy(ev.angleDelta.y, ev.pixelDelta.y, ev.phase)
    }

    FrameAnimation {
        id: frames
        onTriggered: {
            if (Math.abs(flick.contentY - scroller.written) >= 1) {
                stop();
                return;
            }
            scroller.destination = flick.clampContentY(scroller.destination);
            const rest = scroller.destination - flick.contentY;
            const done = Math.abs(rest) < 0.5;
            scroller.moveTo(done ? scroller.destination : flick.contentY + rest * (1 - Math.exp(-frameTime / scroller.tau)));
            if (done)
                stop();
        }
    }
}
