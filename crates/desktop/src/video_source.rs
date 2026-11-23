//! The video player's source (`VideoSource`): the file a video plays from, handed to Qt's player as a device. The file
//! is the catalog's `disk_path` of it, which `video_player` opens and hands over open. Only the GUI thread touches it.

use crate::bridges::video_player_bridge::ffi::VideoPlayer;
use crate::bridges::video_source_state::VideoSourceRust;
use crate::rows::file_id_of;
use crate::video_player;

use crate::bridges::video_source_bridge::qobject;

use core::pin::Pin;

use cxx_qt::CxxQtType;

/// The player, when one is attached.
fn player_of(rust: &mut VideoSourceRust) -> Option<Pin<&mut VideoPlayer>> {
    if rust.player.is_null() { None } else { Some(rust.player.pin_mut()) }
}

impl qobject::VideoSource {
    /// # Safety
    /// As `bridges::video_player_bridge::ffi::new_video_player`: `player` is a live `QMediaPlayer` or null.
    #[expect(unsafe_code, reason = "FFI: attaches the Qt media player the caller keeps alive")]
    pub(crate) unsafe fn attach(mut self: Pin<&mut Self>, player: *mut qobject::QMediaPlayer) {
        // SAFETY: the caller's contract; the C++ side keeps only a weak reference to the player.
        let player = unsafe { crate::bridges::video_player_bridge::ffi::new_video_player(player) };
        self.as_mut().rust_mut().get_mut().player = player;
    }

    pub(crate) fn play(mut self: Pin<&mut Self>, file_id: f64) {
        let Some(file) = file_id_of(file_id) else {
            if let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) {
                player.clear();
            }
            return;
        };
        let path = crate::app::get().catalogs().file_path(file);
        let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) else { return };
        match path {
            Ok(path) => video_player::play_path(player, &path),
            Err(err) => {
                pg_log::warn("video", &format!("file {}: {err:#}", file.get()));
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
