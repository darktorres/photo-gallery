//! `AnimationPlayer`: plays an animated GIF, APNG or WebP into a `VideoOutput`'s sink, so animations show like
//! videos (same fit and turns) while looping as they should (the animation decoder's own notes say why not Qt's player). The
//! loop itself is `pg_render::player`'s; this file is the Qt side: the source and the sink, and the thread-safety of
//! the frame sink's calls.

#[cxx_qt::bridge]
pub mod qobject {
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

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        /// The catalog's file played (its id); 0 stops. The file's path is the app's (`App::file_path`).
        #[qproperty(i64, file_id, cxx_name = "fileId", READ, WRITE = set_file_id_and_play, NOTIFY, FINAL)]
        /// False pauses.
        #[qproperty(bool, playing, READ, WRITE = set_playing_and_apply, NOTIFY, FINAL)]
        type AnimationPlayer = super::AnimationPlayerRust;
    }

    unsafe extern "RustQt" {
        fn set_file_id_and_play(self: Pin<&mut AnimationPlayer>, file_id: i64);
        fn set_playing_and_apply(self: Pin<&mut AnimationPlayer>, playing: bool);

        /// Where the frames go: a `VideoOutput`'s `videoSink`.
        #[qinvokable]
        #[cxx_name = "setSink"]
        unsafe fn set_sink(self: Pin<&mut AnimationPlayer>, sink: *mut QVideoSink);
    }
}

use core::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::CxxQtType;
use pg_model::ids::FileId;
use pg_render::player::{Player, Sink};

pub struct AnimationPlayerRust {
    file_id: i64,
    playing: bool,
    sink: Option<UniquePtr<qobject::SinkRef>>,
    worker: Option<Player<Frames>>,
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { file_id: 0, playing: true, sink: None, worker: None }
    }
}

impl qobject::AnimationPlayer {
    fn set_file_id_and_play(mut self: Pin<&mut Self>, file_id: i64) {
        if file_id == self.file_id {
            return;
        }
        self.as_mut().rust_mut().file_id = file_id;
        self.as_mut().file_id_changed();
        if !self.playing {
            self.as_mut().rust_mut().playing = true;
            self.as_mut().playing_changed();
        }
        self.restart();
    }

    fn set_playing_and_apply(mut self: Pin<&mut Self>, playing: bool) {
        if playing == self.playing {
            return;
        }
        self.as_mut().rust_mut().playing = playing;
        if let Some(w) = &self.worker {
            w.set_paused(!playing);
        }
        self.playing_changed();
    }

    /// # Safety
    /// `sink` is a live `QVideoSink` (from QML: `videoOut.videoSink`), or null.
    unsafe fn set_sink(mut self: Pin<&mut Self>, sink: *mut qobject::QVideoSink) {
        // SAFETY: the caller passes a live sink or null; the reference made of it is weak (QPointer).
        let sink = (!sink.is_null()).then(|| unsafe { qobject::new_sink_ref(sink) });
        self.as_mut().rust_mut().sink = sink;
        self.restart();
    }

    /// Stops the animation playing, then starts the file, if any.
    fn restart(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().worker = None;
        let file_id = self.file_id;
        if file_id <= 0 {
            return;
        }
        let Some(sink) = &self.sink else { return };
        let path = match crate::app::get().file_path(FileId::new(file_id)) {
            Ok(path) => path,
            Err(err) => {
                crate::panic_log::note(&format!("animation {file_id}: {err:#}"));
                return;
            }
        };
        let (frames, paused) = (Arc::new(Frames(sink.frame_sink())), !self.playing);
        self.as_mut().rust_mut().worker = Some(Player::start(path, frames, paused));
    }
}

/// One animation's frame sink, used from its thread.
struct Frames(UniquePtr<qobject::FrameSink>);

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
