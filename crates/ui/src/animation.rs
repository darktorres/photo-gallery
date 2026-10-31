//! `AnimationPlayer`: plays an animated GIF, APNG or WebP into a `VideoOutput`'s sink, so animations show like
//! videos (same fit and turns) while looping as they should (`pg_core::animation` says why not Qt's player).
//! A thread per animation decodes the frames and keeps each one's delay. It loops forever, as most viewers do in
//! practice: a GIF's loop count isn't honoured.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!(<QtMultimedia/QVideoSink>);
        type QVideoSink;

        include!("pg-ui/cpp/frame_sink.h");
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
        /// The file played (a path); empty stops.
        #[qproperty(QString, source, READ, WRITE = set_source_and_play, NOTIFY)]
        /// False pauses.
        #[qproperty(bool, playing, READ, WRITE = set_playing_and_apply, NOTIFY)]
        type AnimationPlayer = super::AnimationPlayerRust;
    }

    unsafe extern "RustQt" {
        fn set_source_and_play(self: Pin<&mut AnimationPlayer>, source: QString);
        fn set_playing_and_apply(self: Pin<&mut AnimationPlayer>, playing: bool);

        /// Where the frames go: a `VideoOutput`'s `videoSink`.
        #[qinvokable]
        #[cxx_name = "setSink"]
        unsafe fn set_sink(self: Pin<&mut AnimationPlayer>, sink: *mut QVideoSink);
    }
}

use core::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cxx::UniquePtr;
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use pg_core::animation;

use crate::state;

pub struct AnimationPlayerRust {
    source: QString,
    playing: bool,
    sink: Option<UniquePtr<qobject::SinkRef>>,
    worker: Option<Worker>,
}

impl Default for AnimationPlayerRust {
    fn default() -> Self {
        Self { source: QString::default(), playing: true, sink: None, worker: None }
    }
}

impl qobject::AnimationPlayer {
    fn set_source_and_play(mut self: Pin<&mut Self>, source: QString) {
        if source == self.source {
            return;
        }
        self.as_mut().rust_mut().source = source;
        self.as_mut().source_changed();
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

    /// Stops the animation playing, then starts the source, if any.
    fn restart(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().worker = None;
        let path = self.source.to_string();
        if path.is_empty() {
            return;
        }
        let Some(sink) = &self.sink else { return };
        let (frames, paused) = (Frames(sink.frame_sink()), !self.playing);
        self.as_mut().rust_mut().worker = Some(Worker::start(path, frames, paused));
    }
}

/// One animation's frame sink, used from its thread.
struct Frames(UniquePtr<qobject::FrameSink>);

// SAFETY: `FrameSink::present` may be called from any thread (cpp/frame_sink.cpp); `close` is called on the GUI
// thread, by `Worker::drop`.
unsafe impl Send for Frames {}
// SAFETY: as above; both methods take `&self` and touch the sink only on its own thread.
unsafe impl Sync for Frames {}

struct Control {
    stop: bool,
    paused: bool,
}

type Shared = Arc<(Mutex<Control>, Condvar)>;

/// The thread playing one animation; dropping it stops it.
struct Worker {
    control: Shared,
    frames: Arc<Frames>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn start(path: String, frames: Frames, paused: bool) -> Worker {
        let control: Shared = Arc::new((Mutex::new(Control { stop: false, paused }), Condvar::new()));
        let frames = Arc::new(frames);
        let thread = std::thread::Builder::new()
            .name("animation".into())
            .spawn({
                let (control, frames) = (control.clone(), frames.clone());
                move || play(&path, &frames, &control)
            })
            .ok();
        Worker { control, frames, thread }
    }

    fn set_paused(&self, paused: bool) {
        let (lock, wake) = &*self.control;
        lock.lock().unwrap().paused = paused;
        wake.notify_all();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.control;
        lock.lock().unwrap().stop = true;
        wake.notify_all();
        // At most one frame's decoding away; after it, nothing more is sent.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.frames.0.close();
    }
}

/// Loops over the animation's frames until stopped. Each frame is shown when the previous one's delay is over
/// (decoding the next counts toward it). An unreadable file, or one whose first frame fails, shows nothing: the
/// viewer keeps the poster.
fn play(path: &str, frames: &Frames, control: &Shared) {
    let Ok(bytes) = std::fs::read(state::long_path(path)) else { return };
    let bytes: Arc<[u8]> = bytes.into();
    let mut due = Instant::now();
    loop {
        let Ok(pass) = animation::frames(bytes.clone()) else { return };
        let mut shown = false;
        // A frame that fails to decode ends the pass: what came before it still loops.
        for frame in pass.map_while(Result::ok) {
            if !wait_until(control, &mut due) {
                return;
            }
            frames.0.present(frame.width, frame.height, &frame.rgba);
            shown = true;
            // Late (a slow decode, a busy machine): carry on from now rather than rushing to catch up.
            due = (due + Duration::from_millis(frame.delay_ms.into())).max(Instant::now());
        }
        if !shown {
            return;
        }
    }
}

/// Waits for `due`, and while paused; false once stopped. After a pause the next frame is due at once.
fn wait_until(control: &Shared, due: &mut Instant) -> bool {
    let (lock, wake) = &**control;
    let mut c = lock.lock().unwrap();
    loop {
        if c.stop {
            return false;
        }
        if c.paused {
            c = wake.wait(c).unwrap();
            *due = Instant::now();
            continue;
        }
        let now = Instant::now();
        if now >= *due {
            return true;
        }
        c = wake.wait_timeout(c, *due - now).unwrap().0;
    }
}
