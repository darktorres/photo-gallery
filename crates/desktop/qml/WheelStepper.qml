// A wheel handler that steps through a list by notches: a notch (`Wheel.notch` units of angleDelta) is one step, and a
// touchpad's small deltas add up until they make one. Up is the previous item, so the steps it gives are negative, as the
// viewer and the Yandex preview have always stepped. Run by `just qmltest` through `stepsFor`, since a WheelHandler gets
// no event from QtTest's mouseWheel under the offscreen platform.
import QtQuick

WheelHandler {
    id: stepper

    // The part of a notch that has not made one yet, carried to the next movement.
    property real notches: 0

    // The steps of a movement: positive is the next item, negative the previous.
    signal stepped(int steps)

    // Drops the part of a notch kept from earlier movements, when the list or the picture changes.
    function reset() {
        stepper.notches = 0;
    }

    // The whole steps a vertical wheel movement of `angleY` makes, with the rest kept for the next movement.
    function stepsFor(angleY: real): int {
        stepper.notches += angleY / Wheel.notch;
        const n = Math.trunc(stepper.notches);
        stepper.notches -= n;
        return -n;
    }

    onWheel: ev => {
        const steps = stepper.stepsFor(ev.angleDelta.y);
        if (steps !== 0)
            stepper.stepped(steps);
    }
}
