//! The animation player's loop: a thread per animation decodes its frames (`pg_codec::animation`) and hands each one to a
//! [`Sink`] when its delay is over. Looping forever, as most viewers do in practice: a GIF's loop count isn't honoured.
//! The sink is the caller's (the viewer's video sink, in the desktop); this module knows nothing of it.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pg_codec::animation;

/// Where the frames go. `present` may be called from the player's thread; `close` once, after the last `present`,
/// when the player stops.
pub trait Sink: Send + Sync + 'static {
    fn present(&self, width: u32, height: u32, rgba: &[u8]);
    fn close(&self) {}
}

struct Control {
    stop: bool,
    paused: bool,
}

type Shared = Arc<(Mutex<Control>, Condvar)>;

/// The thread playing one animation; dropping it stops it, and closes its sink.
pub struct Player<S: Sink> {
    control: Shared,
    sink: Arc<S>,
    thread: Option<JoinHandle<()>>,
}

impl<S: Sink> Player<S> {
    /// Plays the animation file at `path` into `sink`, from the start, paused or not. A file that can't be read or
    /// decoded shows nothing.
    pub fn start(path: String, sink: Arc<S>, paused: bool) -> Player<S> {
        let control: Shared = Arc::new((Mutex::new(Control { stop: false, paused }), Condvar::new()));
        let thread = std::thread::Builder::new()
            .name("animation".into())
            .spawn({
                let (control, sink) = (control.clone(), sink.clone());
                move || play(&path, &*sink, &control)
            })
            .ok();
        Player { control, sink, thread }
    }

    pub fn set_paused(&self, paused: bool) {
        let (lock, wake) = &*self.control;
        lock.lock().unwrap_or_else(PoisonError::into_inner).paused = paused;
        wake.notify_all();
    }
}

impl<S: Sink> Drop for Player<S> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.control;
        lock.lock().unwrap_or_else(PoisonError::into_inner).stop = true;
        wake.notify_all();
        // At most one frame's decoding away; after it, nothing more is sent.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.sink.close();
    }
}

/// Loops over the animation's frames until stopped. Each frame is shown when the previous one's delay is over
/// (decoding the next counts toward it). An unreadable file, or one whose first frame fails, shows nothing: the
/// viewer keeps the poster.
fn play(path: &str, sink: &impl Sink, control: &Shared) {
    let Ok(bytes) = std::fs::read(path) else { return };
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
            sink.present(frame.width, frame.height, &frame.rgba);
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
    let mut c = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if c.stop {
            return false;
        }
        if c.paused {
            c = wake.wait(c).unwrap_or_else(PoisonError::into_inner);
            *due = Instant::now();
            continue;
        }
        let now = Instant::now();
        if now >= *due {
            return true;
        }
        c = wake.wait_timeout(c, *due - now).unwrap_or_else(PoisonError::into_inner).0;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Records the sizes of the frames it is given, and whether it was closed.
    #[derive(Default)]
    struct Record {
        presented: Mutex<Vec<(u32, u32)>>,
        closed: Mutex<bool>,
    }

    impl Sink for Record {
        fn present(&self, width: u32, height: u32, _rgba: &[u8]) {
            self.presented.lock().unwrap().push((width, height));
        }
        fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    fn wait_for(f: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < until, "the player showed nothing in 10 s");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn loops_the_frames_and_closes_the_sink_once_dropped() {
        let dir = std::env::temp_dir().join(format!("pg-player-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("loop.gif");
        std::fs::write(&file, pg_testkit::gif(3).unwrap()).unwrap();

        let sink = Arc::new(Record::default());
        let player = Player::start(file.to_string_lossy().into_owned(), sink.clone(), false);
        // Three frames a pass, so seeing four shows the loop started again.
        wait_for(|| sink.presented.lock().unwrap().len() > 3);
        assert!(sink.presented.lock().unwrap().iter().all(|&size| size == (4, 4)));
        drop(player);
        assert!(*sink.closed.lock().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_shows_nothing() {
        let sink = Arc::new(Record::default());
        let player = Player::start("no-such-animation.gif".into(), sink.clone(), false);
        drop(player);
        assert!(sink.presented.lock().unwrap().is_empty());
        assert!(*sink.closed.lock().unwrap());
    }
}
