// The status bar (StatusBar.qml): the pass's "N failed" is a link to the inbox only while the inbox lists something of
// the pass (Notices' `passes` bit, `failedLinked`): then it is underlined, a link to accessibility, and a click asks for
// the failed files; otherwise it is plain text, a click does nothing, and its tooltip says it isn't recorded yet. An info
// toast goes by itself unless the screenshots hold it. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "StatusBar"
    visible: true
    when: windowShown
    width: 1000
    height: 200

    Component {
        id: barC
        StatusBar {
            selection: ""
            writeStatus: ""
            writeFailed: false
            toast: ""
            toastLevel: 0
            unread: 0
            status: "Importing 1 of 4"
            failed: "2 failed"
            failedLinked: false
            busy: true
            paused: false
            progress: 0.25
            devMode: false
        }
    }

    SignalSpy { id: failedSpy; signalName: "failedRequested" }

    function bar(linked: bool): var {
        const b = createTemporaryObject(barC, tc, { failedLinked: linked })
        verify(b !== null)
        failedSpy.target = b
        failedSpy.clear()
        return b
    }

    function test_the_failed_count_links_only_with_the_passs_bit() {
        const b = bar(false)
        const failed = findChild(b, "statusFailed")
        verify(failed.visible)
        verify(!failed.font.underline)
        compare(failed.Accessible.role, Accessible.StaticText)
        waitForRendering(b)
        mouseClick(failed)
        compare(failedSpy.count, 0, "nothing listed: the click does nothing")
        // The inbox lists the pass's ledger now: the same words become the link.
        b.failedLinked = true
        verify(failed.font.underline)
        compare(failed.Accessible.role, Accessible.Link)
        mouseClick(failed)
        compare(failedSpy.count, 1)
        // And back, when the entry goes.
        b.failedLinked = false
        mouseClick(failed)
        compare(failedSpy.count, 1)
    }

    function test_the_link_starts_live_when_the_bit_is_set() {
        const b = bar(true)
        const failed = findChild(b, "statusFailed")
        verify(failed.font.underline)
        waitForRendering(b)
        mouseClick(failed)
        compare(failedSpy.count, 1)
    }

    function test_no_failures_show_no_count() {
        const b = bar(true)
        b.failed = ""
        verify(!findChild(b, "statusFailed").visible)
    }

    // An info toast's timer runs, unless the toast is held (the screenshots); a warning's never runs.
    function test_an_info_toast_goes_by_itself_unless_held() {
        const b = bar(false)
        const timer = findChild(b, "toastTimer")
        verify(!timer.running)
        b.toast = "Cropped copies saved"
        verify(timer.running)
        b.toastHeld = true
        verify(!timer.running)
        b.toastHeld = false
        b.toastLevel = 1
        verify(!timer.running)
    }
}
