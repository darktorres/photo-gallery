// A video or an animation, fitted to the window and turned by the catalog's turns (Qt's players show the file as
// stored). Videos play with Qt Multimedia on our FFmpeg; animated GIF, APNG and WebP with our AnimationPlayer
// (src/animation.rs), into the same VideoOutput. The transport (VideoTransport) is over it, for videos. The root
// (Viewer.qml) keeps the state and the signals; this reports the players' frames, sources and errors back to it.
pragma ComponentBehavior: Bound

import QtMultimedia
import QtQuick
import pg.gallery

Item {
    id: media

    // Whether the turns swap the picture's width and height (the Viewport's swapsAxes, called by the viewer).
    required property bool swapsAxes
    required property int kind
    required property bool still
    required property bool zen
    required property bool playsVideo
    required property bool playsAnimation
    required property int fw
    required property int fh
    required property int playerTurns
    // Each change asks for a play or pause (Viewer.playRequests counts them).
    required property int playRequests
    onPlayRequestsChanged: togglePlay()
    // The file (its catalog id): a video plays it as a device (VideoSource), an animation is read by AnimationPlayer.
    required property int fileId
    required property bool muted
    required property real volume
    // The height of the rating bar, which the transport sits above.
    required property real barHeight

    readonly property real playScale: fw > 0 && fh > 0 ? Math.min(width / fw, height / fh) : 0

    // From the players: a frame was drawn; the source changed (the poster comes back); the player's error.
    signal frameDrawn()
    signal sourceSwitched()
    signal playerFailed(string text)
    // The transport asks: play or pause, mute, the volume.
    signal playAsked()
    signal muteAsked()
    signal volumeAsked(real value)

    // Play or pause the video or animation shown.
    function togglePlay() {
        if (kind === GalleryModel.Animated)
            animPlayer.playing = !animPlayer.playing
        else if (player.playbackState === MediaPlayer.PlayingState)
            player.pause()
        else
            player.play()
    }

    // The video's file, played as a device (src/video_source.rs): it is opened when the video is shown and stopped when
    // it isn't. The player's own source is only what the device sets.
    function openVideo() {
        if (media.playsVideo)
            video.play(media.fileId)
        else
            video.stop()
    }
    onPlaysVideoChanged: openVideo()
    onFileIdChanged: openVideo()
    Component.onCompleted: openVideo()

    // Fitted (up or down), turned by the catalog's turns.
    Item {
        visible: !media.still && media.playScale > 0
        width: media.fw * media.playScale
        height: media.fh * media.playScale
        anchors.centerIn: parent
        Item {
            anchors.centerIn: parent
            width: media.swapsAxes ? parent.height : parent.width
            height: media.swapsAxes ? parent.width : parent.height
            rotation: media.playerTurns * 90
            VideoOutput {
                id: videoOut
                anchors.fill: parent
                fillMode: VideoOutput.PreserveAspectFit
            }
        }
    }

    MediaPlayer {
        id: player
        videoOutput: videoOut
        audioOutput: AudioOutput {
            muted: media.muted
            volume: media.volume
        }
        loops: MediaPlayer.Infinite
        onSourceChanged: {
            media.sourceSwitched()
            if (media.playsVideo)
                play()
        }
        onErrorChanged: media.playerFailed(errorString)
    }

    VideoSource {
        id: video
        Component.onCompleted: attach(player)
    }

    AnimationPlayer {
        id: animPlayer
        fileId: media.playsAnimation ? media.fileId : 0
        onFileIdChanged: media.sourceSwitched()
        Component.onCompleted: setSink(videoOut.videoSink)
    }

    // The first frame: the poster stays until then.
    Connections {
        target: videoOut.videoSink
        function onVideoFrameChanged() {
            media.frameDrawn()
        }
    }

    VideoTransport {
        playing: player.playbackState === MediaPlayer.PlayingState
        position: player.position
        duration: player.duration
        seekable: player.seekable
        onSeekAsked: (ms) => player.position = ms
        showTransport: media.kind === GalleryModel.Video && !media.zen
        barHeight: media.barHeight
        muted: media.muted
        volume: media.volume
        onPlayAsked: media.playAsked()
        onMuteAsked: media.muteAsked()
        onVolumeAsked: (value) => media.volumeAsked(value)
    }
}
