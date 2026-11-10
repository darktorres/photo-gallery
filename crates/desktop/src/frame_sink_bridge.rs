//! The cxx-qt bridge of [`crate::frame_sink`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

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
