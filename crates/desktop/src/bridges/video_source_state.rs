//! The state of the `VideoSource` QObject: the bridge's backing struct (the player's device).

use cxx::UniquePtr;

use crate::bridges::video_player_bridge::ffi::VideoPlayer;

/// The player's device.
pub struct VideoSourceRust {
    /// Null until `attach`.
    pub(crate) player: UniquePtr<VideoPlayer>,
}

impl Default for VideoSourceRust {
    fn default() -> Self {
        Self { player: UniquePtr::null() }
    }
}
