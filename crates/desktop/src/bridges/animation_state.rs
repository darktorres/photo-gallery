//! The state of the `AnimationPlayer` QObject: the bridge's backing struct (the file played, whether it plays, the sink
//! and the player).

use cxx::UniquePtr;
use pg_render::player::Player;

use crate::bridges::frame_sink_bridge::ffi::SinkRef;
use crate::frame_sink::Frames;

/// The file played, its sink, and the player playing it into the sink.
pub struct AnimationPlayerRust {
    pub(crate) file_id: i64,
    pub(crate) playing: bool,
    pub(crate) sink: Option<UniquePtr<SinkRef>>,
    pub(crate) worker: Option<Player<Frames>>,
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { file_id: 0, playing: true, sink: None, worker: None }
    }
}
