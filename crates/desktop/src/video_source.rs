//! The video player's source (`VideoSource`): the file a video plays from, handed to Qt's player as a device. The file
//! is the catalog's `disk_path` of it, which `video_player` opens and hands over open. Only the GUI thread touches it.

pub(crate) mod state;

use crate::video_player;
use crate::video_player_bridge::ffi::VideoPlayer;
use state::VideoSourceRust;

use crate::video_source_bridge::qobject;

use core::pin::Pin;

use cxx_qt::CxxQtType;
use pg_model::ids::FileId;

/// The player, when one is attached.
fn player_of(rust: &mut VideoSourceRust) -> Option<Pin<&mut VideoPlayer>> {
    if rust.player.is_null() { None } else { Some(rust.player.pin_mut()) }
}

impl qobject::VideoSource {
    /// # Safety
    /// As `video_player_bridge::ffi::new_video_player`: `player` is a live `QMediaPlayer` or null.
    pub(crate) unsafe fn attach(mut self: Pin<&mut Self>, player: *mut qobject::QMediaPlayer) {
        // SAFETY: the caller's contract; the C++ side keeps only a weak reference to the player.
        let player = unsafe { crate::video_player_bridge::ffi::new_video_player(player) };
        self.as_mut().rust_mut().get_mut().player = player;
    }

    pub(crate) fn play(mut self: Pin<&mut Self>, file_id: i64) {
        let path = crate::app::get().catalogs().file_path(FileId::new(file_id));
        let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) else { return };
        match path {
            Ok(path) => video_player::play_path(player, &path),
            Err(err) => {
                pg_log::warn("video", &format!("file {file_id}: {err:#}"));
                player.clear();
            }
        }
    }

    pub(crate) fn stop(mut self: Pin<&mut Self>) {
        if let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) {
            player.clear();
        }
    }
}
