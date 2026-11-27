//! `AnimationPlayer`: plays an animated GIF, APNG or WebP into a `VideoOutput`'s sink, so animations show like
//! videos (same fit and turns) while looping as they should (the animation decoder's own notes say why not Qt's player). The
//! loop itself is `pg_render::player`'s; this file is the Qt side: the source and the sink. The sink's FFI and the
//! thread-safety of its calls are `frame_sink`'s.
use crate::frame_sink::Frames;

use crate::bridges::animation_bridge::qobject;

use core::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use pg_model::ids::FileId;
use pg_render::player::{Player, Sink};

/// An animation's sink: its frames go to the video sink (`Frames`), and the player's failures to the `AnimationPlayer`'s
/// `playerError`, on the window's thread. A failure of a file that is no longer the one played is dropped there.
pub(crate) struct Watched {
    frames: Frames,
    player: CxxQtThread<qobject::AnimationPlayer>,
    file_id: i64,
}

impl Sink for Watched {
    fn present(&self, width: u32, height: u32, rgba: &[u8]) {
        self.frames.present(width, height, rgba);
    }

    fn failed(&self, error: &str) {
        let (file_id, text) = (self.file_id, error.to_string());
        let _ = self.player.queue(move |player| player.animation_failed(file_id, &text));
    }

    fn close(&self) {
        self.frames.close();
    }
}

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
    #[expect(
        unsafe_code,
        reason = "FFI: the video sink pointer the Qt media player hands over, kept alive by the caller"
    )]
    pub(crate) unsafe fn set_sink(mut self: Pin<&mut Self>, sink: *mut qobject::QVideoSink) {
        // SAFETY: the caller passes a live sink or null; the reference made of it is weak (QPointer).
        let sink = (!sink.is_null()).then(|| unsafe { crate::bridges::frame_sink_bridge::ffi::new_sink_ref(sink) });
        self.as_mut().rust_mut().sink = sink;
        self.restart();
    }

    /// The player reported a failure for `file_id` (`Watched::failed`): what it says is the player's error.
    fn animation_failed(mut self: Pin<&mut Self>, file_id: i64, text: &str) {
        if file_id == self.file_id {
            self.as_mut().set_player_error_text(text);
        }
    }

    /// Sets the player's error text (`""` for none), signalling only a change.
    fn set_player_error_text(mut self: Pin<&mut Self>, text: &str) {
        if self.player_error.to_string() != text {
            self.as_mut().rust_mut().player_error = QString::from(&text.to_string());
            self.as_mut().player_error_changed();
        }
    }

    /// Stops the animation playing, then starts the file, if any. A file that can't be found or played is said in
    /// `playerError`.
    fn restart(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().worker = None;
        self.as_mut().set_player_error_text("");
        let file_id = self.file_id;
        if file_id <= 0 || self.sink.is_none() {
            return;
        }
        let path = match crate::app::get().catalogs().file_path(FileId::new(file_id)) {
            Ok(path) => path,
            Err(err) => {
                let text = format!("{err:#}");
                pg_log::warn("animation", &format!("file {file_id}: {text}"));
                self.as_mut().set_player_error_text(&text);
                return;
            }
        };
        let Some(sink) = &self.sink else { return };
        let frames = Watched { frames: Frames::of(sink), player: self.qt_thread(), file_id };
        let paused = !self.playing;
        self.as_mut().rust_mut().worker = Some(Player::start(path, Arc::new(frames), paused));
    }
}
