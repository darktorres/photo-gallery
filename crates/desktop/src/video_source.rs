//! The video player's source (`VideoSource`): the file a video plays from, handed to Qt's player as a device. The file
//! is the catalog's exact path; how it reaches the player on each platform is `video_player`'s. Only the GUI thread
//! touches it.

mod state;

use crate::video_player::{self, ffi::VideoPlayer};
use state::VideoSourceRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtMultimedia/QMediaPlayer>);
        type QMediaPlayer = crate::video_player::ffi::QMediaPlayer;
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

use core::pin::Pin;

use cxx_qt::CxxQtType;
use pg_model::ids::FileId;

/// The player, when one is attached.
fn player_of(rust: &mut VideoSourceRust) -> Option<Pin<&mut VideoPlayer>> {
    if rust.player.is_null() { None } else { Some(rust.player.pin_mut()) }
}

impl qobject::VideoSource {
    /// # Safety
    /// As `video_player::ffi::new_video_player`: `player` is a live `QMediaPlayer` or null.
    unsafe fn attach(mut self: Pin<&mut Self>, player: *mut qobject::QMediaPlayer) {
        // SAFETY: the caller's contract; the C++ side keeps only a weak reference to the player.
        let player = unsafe { video_player::ffi::new_video_player(player) };
        self.as_mut().rust_mut().get_mut().player = player;
    }

    fn play(mut self: Pin<&mut Self>, file_id: i64) {
        let path = crate::app::get().catalogs().file_path(FileId::new(file_id));
        let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) else { return };
        match path {
            Ok(path) => video_player::play_path(player, &path),
            Err(err) => {
                crate::log::note(&format!("video {file_id}: {err:#}"));
                player.clear();
            }
        }
    }

    fn stop(mut self: Pin<&mut Self>) {
        if let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) {
            player.clear();
        }
    }
}
