// Mouse-wheel scrolling for the Flickable (ListView, …) it is declared in, that keeps up with a fast wheel.
//
// Qt Quick's own wheel scrolling (QQuickFlickable::wheelEvent, Qt 6.6+) eases each notch over 300 ms, but a new
// notch restarts the ease from wherever the last one had got to and drops the rest of it: the faster the wheel
// spins, the less of each notch is travelled, so speed tops out near 23 notches' worth of pixels per second
// however fast it spins (QTBUG-129948, QTBUG-116388). Here a notch moves a destination instead, as in Qt Widgets
// (QAbstractSlider::scrollByDelta adds every notch in full), and the view eases toward it each frame on Qt's own
// curve: a lone notch moves exactly as before, a burst of notches travels all of them.
import QtQuick

WheelHandler {
    id: scroller

    // Pixels per wheel line; a notch (120 units of angleDelta) is the system's lines per notch of them. Qt Quick's own
    // is 24 (QQuickFlickable::wheelEvent).
    property real lineHeight: 24

    // The Flickable. A handler declared in one is moved onto its contentItem (QQuickFlickablePrivate::addPointerHandler),
    // which hit-tests as the whole view (QQuickFlickableContentItem::contains) and is reached before the Flickable's
    // own wheelEvent(): so parent is the contentItem, and the view is the first Flickable above it.
    function view(): Flickable {
        let p = parent
        while (p && !(p instanceof Flickable))
            p = p.parent
        return p as Flickable
    }
    // Where the view is heading, and the contentY last written, to notice anything else moving it (a scrollbar
    // drag, a key, positionViewAtIndex), which ends the scroll. Qt itself moves it by less than a pixel: a Flickable
    // that is not "moving" (and writing contentY is not) snaps to a whole pixel whenever its contentHeight changes
    // (QQuickFlickable::setContentHeight -> QQuickFlickablePrivate::fixup), which a ListView's does as it creates rows.
    property real destination: 0
    property real written: 0
    // Qt eases a notch with OutExpo over 300 ms: 1 - 2^(-10 t / 0.3 s), i.e. an exponential approach with this time
    // constant. Chasing the destination with it gives a lone notch the same motion.
    readonly property real tau: 0.3 / (10 * Math.LN2)

    // Plain wheel only: Ctrl+wheel and the like stay with other handlers (modifiers must match exactly).
    acceptedModifiers: Qt.NoModifier
    orientation: Qt.Vertical

    // contentY's range, as QQuickFlickable / QQuickItemView compute it (-minYExtent … -maxYExtent). A ListView's
    // contentHeight and originY are estimates that firm up as rows are created, so this is re-read every frame.
    function clamp(y) {
        const v = view()
        const top = v.originY - v.topMargin
        const bottom = Math.max(top, v.originY + v.contentHeight + v.bottomMargin - v.height)
        return Math.max(top, Math.min(bottom, y))
    }

    function moveTo(y) {
        written = y
        view().contentY = y  // also stops a flick of the Flickable's own (setContentY resets its timeline)
    }

    onWheel: (ev) => {
        if (!frames.running)
            destination = view().contentY
        // Pixel deltas from the platform (touchpads, on some systems): follow them as they come, which is what they
        // are for. Qt takes this path under the same conditions, except that it cannot see the event's source here.
        if (ev.pixelDelta.y !== 0 && ev.phase !== Qt.NoScrollPhase) {
            frames.stop()
            moveTo(clamp(destination - ev.pixelDelta.y))
            return
        }
        destination = clamp(destination - ev.angleDelta.y / 120 * Application.styleHints.wheelScrollLines * lineHeight)
        if (!frames.running) {
            written = view().contentY
            frames.start()
        }
    }

    property FrameAnimation frames: FrameAnimation {
        onTriggered: {
            const v = scroller.view()
            if (Math.abs(v.contentY - scroller.written) >= 1) {
                stop()
                return
            }
            scroller.destination = scroller.clamp(scroller.destination)
            const rest = scroller.destination - v.contentY
            const done = Math.abs(rest) < 0.5
            scroller.moveTo(done ? scroller.destination : v.contentY + rest * (1 - Math.exp(-frameTime / scroller.tau)))
            if (done)
                stop()
        }
    }
}
