//! `AnimationPlayer`: plays an animated GIF, APNG or WebP into a `VideoOutput`'s sink, so animations show like
//! videos (same fit and turns) while looping as they should (the animation decoder's own notes say why not Qt's player). The
//! loop itself is `pg_render::player`'s; this file is the Qt side: the source and the sink. The sink's FFI and the
//! thread-safety of its calls are `frame_sink`'s.
use crate::frame_sink::Frames;

use crate::bridges::animation_bridge::qobject;

use core::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use pg_client::image::ImageRequest;
use pg_model::ids::FileId;
use pg_render::player::{Player, Sink, Size, read_file, shared};

/// An animation's sink: its frames go to the video sink (`Frames`), and the player's failures to `report`, which its owner
/// makes on the window's thread (an `AnimationPlayer`'s `playerError`, or a result player's): a failure of a run that is no
/// longer the one played is dropped there.
pub(crate) struct Watched {
    frames: Frames,
    report: Box<dyn Fn(String) + Send + Sync>,
}

impl Sink for Watched {
    fn present(&self, width: u32, height: u32, rgba: &[u8]) {
        self.frames.present(width, height, rgba);
    }

    fn failed(&self, error: &str) {
        (self.report)(error.to_string());
    }

    fn close(&self) {
        self.frames.close();
    }
}

impl qobject::AnimationPlayer {
    /// The height the frames are shown at (0 for the file's own size): restarts the playback at it.
    pub(crate) fn set_height_and_restart(mut self: Pin<&mut Self>, height: i32) {
        if self.height == height {
            return;
        }
        self.as_mut().rust_mut().height = height;
        self.as_mut().height_changed();
        self.restart();
    }

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

    /// The kind of `player`'s error (`qobject::player_error_kind`, cpp/player_error.cpp), as the viewer words it.
    ///
    /// # Safety
    /// `player` is a live `QMediaPlayer` or null (from QML: the viewer's `MediaPlayer`).
    #[expect(
        unsafe_code,
        reason = "FFI: the Qt media player the QML hands over, read on the window's thread while it is alive"
    )]
    pub(crate) unsafe fn player_error_kind_of(&self, player: *mut qobject::QMediaPlayer) -> i32 {
        // SAFETY: the caller passes a live player or null (see above).
        unsafe { qobject::player_error_kind(player) }
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
        let Some(sink) = &self.sink else { return };
        let player = self.qt_thread();
        let report = Box::new(move |text: String| {
            let _ = player.queue(move |player| player.animation_failed(file_id, &text));
        });
        let frames = Watched { frames: Frames::of(sink), report };
        let paused = !self.playing;
        // The path is looked up and the file read on the player's thread, so a screenful of cells never runs a catalog
        // query on the window's: a file that can't be found is reported (and logged) as one that can't be read.
        let load = move || {
            let path = crate::app::get().catalogs().file_path(FileId::new(file_id)).map_err(|err| {
                let text = format!("{err:#}");
                pg_log::warn("animation", &format!("file {file_id}: {text}"));
                text
            })?;
            read_file(path)()
        };
        let size = u32::try_from(self.height).ok().filter(|&px| px > 0).map_or(Size::Native, Size::Height);
        self.as_mut().rust_mut().worker = Some(Player::start_sized(load, Arc::new(frames), paused, size));
    }
}

impl qobject::ResultAnimationPlayer {
    /// The result's URL to play (`image://yandex/<id>/original`): restarts the playback, from the start, playing.
    pub(crate) fn set_url_and_play(mut self: Pin<&mut Self>, url: QString) {
        if self.url.to_string() == url.to_string() {
            return;
        }
        self.as_mut().rust_mut().url = url;
        self.as_mut().url_changed();
        self.restart();
    }

    /// # Safety
    /// `sink` is a live `QVideoSink` (from QML: a `VideoOutput`'s `videoSink`), or null.
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

    /// The height the frames are shown at (0 for the file's own size): restarts the playback at it.
    pub(crate) fn set_height_and_play(mut self: Pin<&mut Self>, height: i32) {
        if self.height == height {
            return;
        }
        self.as_mut().rust_mut().height = height;
        self.as_mut().height_changed();
        self.restart();
    }

    /// A run reported a failure: it is the player's error, unless the run was replaced since (`generation`).
    fn result_failed(mut self: Pin<&mut Self>, generation: u64, text: &str) {
        if generation == self.generation {
            self.as_mut().set_error_text(text);
        }
    }

    /// Sets the player's error text (`""` for none), signalling only a change.
    fn set_error_text(mut self: Pin<&mut Self>, text: &str) {
        if self.player_error.to_string() != text {
            self.as_mut().rust_mut().player_error = QString::from(&text.to_string());
            self.player_error_changed();
        }
    }

    /// Stops the playback, then plays the result's original, if there is one and a sink. The file is taken from the
    /// search's kept download on the player's thread, and never waited for.
    fn restart(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().worker = None;
        self.as_mut().rust_mut().generation += 1;
        let generation = self.generation;
        self.as_mut().set_error_text("");
        let url = self.url.to_string();
        if url.is_empty() || self.sink.is_none() {
            return;
        }
        let id = match ImageRequest::from_url(&url) {
            Some(ImageRequest::Yandex { id, original: true }) => id,
            _ => {
                self.as_mut().set_error_text("it is not a search result's original");
                return;
            }
        };
        let Some(sink) = &self.sink else { return };
        let player = self.qt_thread();
        let report = Box::new(move |text: String| {
            let _ = player.queue(move |player| player.result_failed(generation, &text));
        });
        let frames = Watched { frames: Frames::of(sink), report };
        let load = shared(move || {
            crate::app::get().yandex().kept_image(id).ok_or_else(|| "the result's file is not kept".to_string())
        });
        // A tile's height is a size in pixels (a row's): a non-positive one is the file's own size.
        let size = u32::try_from(self.height).ok().filter(|&px| px > 0).map_or(Size::Native, Size::Height);
        self.as_mut().rust_mut().worker = Some(Player::start_sized(load, Arc::new(frames), false, size));
    }
}
