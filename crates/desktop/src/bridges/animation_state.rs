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
    pub(crate) height: i32,
}

/// The state of a `ResultAnimationPlayer`: the result's URL, the sink, the player playing it into the sink, and the
/// generation of the run (a failure of a run that is no longer the one played is dropped).
pub struct ResultAnimationPlayerRust {
    pub(crate) url: QString,
    pub(crate) sink: Option<UniquePtr<SinkRef>>,
    pub(crate) worker: Option<Player<crate::animation::Watched>>,
    pub(crate) player_error: QString,
    pub(crate) generation: u64,
    pub(crate) height: i32,
}

impl Default for ResultAnimationPlayerRust {
    fn default() -> Self {
        Self {
            url: QString::default(),
            sink: None,
            worker: None,
            player_error: QString::default(),
            generation: 0,
            height: 0,
        }
    }
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { file_id: 0, playing: true, sink: None, worker: None, player_error: QString::default(), height: 0 }
    }
}
