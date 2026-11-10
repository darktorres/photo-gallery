//! The cxx-qt bridge of [`crate::video_source`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::video_source::state::VideoSourceRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtMultimedia/QMediaPlayer>);
        type QMediaPlayer = crate::video_player_bridge::ffi::QMediaPlayer;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type VideoSource = super::VideoSourceRust;
    }

    unsafe extern "RustQt" {
        /// The player the video plays on (QML's `MediaPlayer`). Called once, when the source is made.
        ///
        /// # Safety
        /// `player` is a live `QMediaPlayer` or null (see `new_video_player`).
        #[qinvokable]
        unsafe fn attach(self: Pin<&mut VideoSource>, player: *mut QMediaPlayer);

        /// Plays the video of the catalog's file `file_id` from the start. A file the catalog doesn't have clears the
        /// player instead.
        #[qinvokable]
        fn play(self: Pin<&mut VideoSource>, file_id: i64);

        /// Stops the video: the player's source is cleared.
        #[qinvokable]
        fn stop(self: Pin<&mut VideoSource>);
    }
}
