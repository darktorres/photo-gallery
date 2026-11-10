//! The state of the `AnimationPlayer` QObject: the bridge's backing struct (the file played, whether it plays, the sink
//! and the player).

use cxx::UniquePtr;
use pg_render::player::Player;

use crate::frame_sink::Frames;
use crate::frame_sink::ffi::SinkRef;

/// The file played, its sink, and the player playing it into the sink.
pub struct AnimationPlayerRust {
    pub(super) file_id: i64,
    pub(super) playing: bool,
    pub(super) sink: Option<UniquePtr<SinkRef>>,
    pub(super) worker: Option<Player<Frames>>,
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { file_id: 0, playing: true, sink: None, worker: None }
    }
}
