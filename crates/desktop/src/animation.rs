//! `AnimationPlayer`: plays an animated GIF, APNG or WebP into a `VideoOutput`'s sink, so animations show like
//! videos (same fit and turns) while looping as they should (the animation decoder's own notes say why not Qt's player). The
//! loop itself is `pg_render::player`'s; this file is the Qt side: the source and the sink. The sink's FFI and the
//! thread-safety of its calls are `frame_sink`'s.

pub(crate) mod state;
use crate::frame_sink::Frames;

use crate::animation_bridge::qobject;

use core::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use pg_model::ids::FileId;
use pg_render::player::Player;

impl qobject::AnimationPlayer {
    pub(crate) fn set_file_id_and_play(mut self: Pin<&mut Self>, file_id: i64) {
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

    pub(crate) fn set_playing_and_apply(mut self: Pin<&mut Self>, playing: bool) {
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
    pub(crate) unsafe fn set_sink(mut self: Pin<&mut Self>, sink: *mut qobject::QVideoSink) {
        // SAFETY: the caller passes a live sink or null; the reference made of it is weak (QPointer).
        let sink = (!sink.is_null()).then(|| unsafe { crate::frame_sink_bridge::ffi::new_sink_ref(sink) });
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
        let path = match crate::app::get().catalogs().file_path(FileId::new(file_id)) {
            Ok(path) => path,
            Err(err) => {
                pg_log::warn("animation", &format!("file {file_id}: {err:#}"));
                return;
            }
        };
        let (frames, paused) = (Arc::new(Frames::of(sink)), !self.playing);
        self.as_mut().rust_mut().worker = Some(Player::start(path, frames, paused));
    }
}
