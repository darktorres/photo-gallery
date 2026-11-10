//! The frame sink an animation plays into (cpp/frame_sink.h): a weak reference to a `VideoOutput`'s `QVideoSink`, and
//! the sink's frame queue, which any thread may present to. `Frames` is the player's side of it (`pg_render`'s
//! `Sink`). A bridge with no QObject, below the `AnimationPlayer` bridge and its state, which both name its types.

use cxx::UniquePtr;
use pg_render::player::Sink;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!(<QtMultimedia/QVideoSink>);
        type QVideoSink;

        include!("pg-desktop/cpp/frame_sink.h");
        #[namespace = "pg"]
        type FrameSink;
        #[namespace = "pg"]
        type SinkRef;
        /// A weak reference to `sink`, which must be alive.
        #[namespace = "pg"]
        unsafe fn new_sink_ref(sink: *mut QVideoSink) -> UniquePtr<SinkRef>;
        #[namespace = "pg"]
        fn frame_sink(self: &SinkRef) -> UniquePtr<FrameSink>;
        /// Any thread: queues the frame for the sink's thread.
        #[namespace = "pg"]
        fn present(self: &FrameSink, width: u32, height: u32, rgba: &[u8]);
        /// The sink's thread: frames still queued are dropped.
        #[namespace = "pg"]
        fn close(self: &FrameSink);
    }
}

/// One animation's frame sink, used from its thread.
pub(crate) struct Frames(UniquePtr<ffi::FrameSink>);

impl Frames {
    /// The frame sink of `sink`'s video sink.
    pub(crate) fn of(sink: &ffi::SinkRef) -> Frames {
        Frames(sink.frame_sink())
    }
}

// SAFETY: `FrameSink::present` may be called from any thread (cpp/frame_sink.cpp); `close` is called on the GUI
// thread, by `Player`'s drop.
unsafe impl Send for Frames {}
// SAFETY: as above; both methods take `&self` and touch the sink only on its own thread.
unsafe impl Sync for Frames {}

impl Sink for Frames {
    fn present(&self, width: u32, height: u32, rgba: &[u8]) {
        self.0.present(width, height, rgba);
    }

    fn close(&self) {
        self.0.close();
    }
}
