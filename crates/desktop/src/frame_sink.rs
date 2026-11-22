//! The frame sink an animation plays into (cpp/frame_sink.h): a weak reference to a `VideoOutput`'s `QVideoSink`, and
//! the sink's frame queue, which any thread may present to. `Frames` is the player's side of it (`pg_render`'s
//! `Sink`). A bridge with no QObject, below the `AnimationPlayer` bridge and its state, which both name its types.

use cxx::UniquePtr;
use pg_render::player::Sink;

use crate::frame_sink_bridge::ffi;

/// One animation's frame sink, used from its thread.
pub(crate) struct Frames(UniquePtr<ffi::FrameSink>);

impl Frames {
    /// The frame sink of `sink`'s video sink.
    pub(crate) fn of(sink: &ffi::SinkRef) -> Frames {
        Frames(sink.frame_sink())
    }
}

// SAFETY: `FrameSink::present` and `close` may be called from any thread: the sink's only shared state is an atomic
// flag, and a frame runs on the sink's own thread (cpp/frame_sink.cpp).
#[expect(unsafe_code, reason = "the SAFETY comment below: the sink's methods may run on any thread")]
unsafe impl Send for Frames {}
// SAFETY: as above; both methods take `&self`, and the only state they share is that flag.
#[expect(unsafe_code, reason = "the SAFETY comment above: both methods take `&self`, and share only an atomic flag")]
unsafe impl Sync for Frames {}

impl Sink for Frames {
    fn present(&self, width: u32, height: u32, rgba: &[u8]) {
        self.0.present(width, height, rgba);
    }

    fn close(&self) {
        self.0.close();
    }
}
