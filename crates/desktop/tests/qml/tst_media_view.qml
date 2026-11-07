// The media view's video source (S-17 Unit B): a video plays by its file's id, a new file replaces it, and an item
// that isn't a video stops it. The VideoSource is a stub (tests/qml/stubs) whose state is in VideoLog; the players and
// the decoder are not run here. Run by `just qmltest`.
import QtQuick
import QtTest
import pg.gallery

TestCase {
    id: tc
    name: "MediaView"
    visible: true
    when: windowShown

    // A video of file 7, as the viewer makes it; each test changes what it needs.
    Component {
        id: mediaC
        MediaView {
            width: 800
            height: 600
            swapsAxes: false
            kind: GalleryModel.Video
            still: false
            zen: false
            playsVideo: true
            playsAnimation: false
            fw: 640
            fh: 480
            playerTurns: 0
            playRequests: 0
            fileId: 7
            muted: false
            volume: 1
            barHeight: 0
        }
    }

    function init() {
        VideoLog.reset()
    }

    function test_a_video_plays_its_file() {
        const m = createTemporaryObject(mediaC, tc)
        verify(m !== null)
        compare(VideoLog.playing, true)
        compare(VideoLog.playingFileId, 7)
    }

    function test_a_new_file_plays_the_new_video() {
        const m = createTemporaryObject(mediaC, tc)
        verify(m !== null)
        m.fileId = 9
        compare(VideoLog.playing, true)
        compare(VideoLog.playingFileId, 9)
    }

    function test_leaving_video_stops_it() {
        const m = createTemporaryObject(mediaC, tc)
        verify(m !== null)
        compare(VideoLog.playing, true)
        m.playsVideo = false
        compare(VideoLog.playing, false)
    }

    function test_an_animation_replaces_the_video() {
        const m = createTemporaryObject(mediaC, tc)
        verify(m !== null)
        compare(VideoLog.playing, true)
        m.kind = GalleryModel.Animated
        m.playsVideo = false
        m.playsAnimation = true
        compare(VideoLog.playing, false)
    }
}
