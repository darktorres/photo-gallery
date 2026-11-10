//! The state of the `VideoSource` QObject: the bridge's backing struct (the player's device).

use cxx::UniquePtr;

use crate::video_player::ffi::VideoPlayer;

/// The player's device.
pub struct VideoSourceRust {
    /// Null until `attach`.
    pub(super) player: UniquePtr<VideoPlayer>,
}

impl Default for VideoSourceRust {
    fn default() -> Self {
        Self { player: UniquePtr::null() }
    }
}
