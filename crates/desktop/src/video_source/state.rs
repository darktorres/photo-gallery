//! The state of the `VideoSource` QObject: the bridge's backing struct (the player's device).

use cxx::UniquePtr;

use crate::video_player_bridge::ffi::VideoPlayer;

/// The player's device.
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct VideoSourceRust {
    /// Null until `attach`.
    pub(super) player: UniquePtr<VideoPlayer>,
}

impl Default for VideoSourceRust {
    fn default() -> Self {
        Self { player: UniquePtr::null() }
    }
}
