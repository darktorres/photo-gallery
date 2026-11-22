// WheelStepper: a notch is one step, up is the previous item, a touchpad's small deltas add up until they make a notch,
// and reset() drops the part kept. The tests call stepsFor() with an angle delta rather than sending mouseWheel(), which
// a WheelHandler does not receive under the offscreen platform (see tst_wheellistview.qml). Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "WheelStepper"

    Component {
        id: stepperC
        WheelStepper {}
    }

    function test_a_notch_is_one_step_and_up_is_the_previous_item() {
        const s = createTemporaryObject(stepperC, tc);
        verify(s !== null);
        compare(s.stepsFor(120), -1);
        compare(s.stepsFor(-120), 1);
    }

    function test_several_notches_are_several_steps() {
        const s = createTemporaryObject(stepperC, tc);
        verify(s !== null);
        compare(s.stepsFor(-360), 3);
    }

    function test_small_deltas_add_up_until_they_make_a_notch() {
        const s = createTemporaryObject(stepperC, tc);
        verify(s !== null);
        compare(s.stepsFor(40), 0);
        compare(s.stepsFor(40), 0);
        compare(s.stepsFor(40), -1);
    }

    function test_reset_drops_the_part_of_a_notch_kept() {
        const s = createTemporaryObject(stepperC, tc);
        verify(s !== null);
        compare(s.stepsFor(80), 0);
        s.reset();
        compare(s.stepsFor(80), 0, "the 80 kept before the reset must not make the next notch");
    }
}
