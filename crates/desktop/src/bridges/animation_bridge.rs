//! The cxx-qt bridge of [`crate::animation`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::animation_state::AnimationPlayerRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtMultimedia/QVideoSink>);
        type QVideoSink = crate::bridges::frame_sink_bridge::ffi::QVideoSink;
        include!(<QtMultimedia/QMediaPlayer>);
        type QMediaPlayer = crate::bridges::video_player_bridge::ffi::QMediaPlayer;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("pg-desktop/cpp/player_error.h");
        /// The kind of `player`'s error (cpp/player_error.cpp): 0 none, 1 open, 2 format, 3 any other failure. Null is none.
        ///
        /// # Safety
        /// `player` is a live `QMediaPlayer` or null.
        #[namespace = "pg"]
        unsafe fn player_error_kind(player: *mut QMediaPlayer) -> i32;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The catalog's file played (its id); 0 stops. The file's path is the catalog's (`Catalogs::file_path`).
        #[qproperty(i64, file_id, cxx_name = "fileId", READ, WRITE = set_file_id_and_play, NOTIFY, FINAL)]
        /// False pauses.
        #[qproperty(bool, playing, READ, WRITE = set_playing_and_apply, NOTIFY, FINAL)]
        /// Why the animation can't be played (the file can't be found, read or decoded): empty when it can. Shown in the HUD.
        #[qproperty(QString, player_error, cxx_name = "playerError", READ, NOTIFY, FINAL)]
        type AnimationPlayer = super::AnimationPlayerRust;
    }

    impl cxx_qt::Threading for AnimationPlayer {}

    unsafe extern "RustQt" {
        fn set_file_id_and_play(self: Pin<&mut AnimationPlayer>, file_id: i64);
        fn set_playing_and_apply(self: Pin<&mut AnimationPlayer>, playing: bool);

        /// Where the frames go: a `VideoOutput`'s `videoSink`.
        #[qinvokable]
        #[cxx_name = "setSink"]
        unsafe fn set_sink(self: Pin<&mut AnimationPlayer>, sink: *mut QVideoSink);

        /// The kind of a media player's error, for the viewer's words: 0 none, 1 the file couldn't be opened, 2 its format
        /// isn't supported, 3 any other failure. QML can't compare the player's error enum, so C++ maps it. `player` is the
        /// viewer's video `MediaPlayer`; the animation player does not play it.
        ///
        /// # Safety
        /// `player` is a live `QMediaPlayer` or null (from QML: a `MediaPlayer`).
        #[qinvokable]
        #[cxx_name = "playerErrorKind"]
        unsafe fn player_error_kind_of(self: &AnimationPlayer, player: *mut QMediaPlayer) -> i32;
    }
}
