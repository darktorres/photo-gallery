//! The state of the `AnimationPlayer` QObject: the bridge's backing struct (the file played, whether it plays, the sink
//! and the player).

use cxx::UniquePtr;
use cxx_qt_lib::QString;
use pg_render::player::Player;

use crate::animation::Watched;
use crate::bridges::frame_sink_bridge::ffi::SinkRef;

/// The file played, its sink, and the player playing it into the sink (whose failures are its `playerError`).
pub struct AnimationPlayerRust {
    pub(crate) file_id: i64,
    pub(crate) playing: bool,
    pub(crate) sink: Option<UniquePtr<SinkRef>>,
    pub(crate) worker: Option<Player<Watched>>,
    pub(crate) player_error: QString,
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { file_id: 0, playing: true, sink: None, worker: None, player_error: QString::default() }
    }
}
