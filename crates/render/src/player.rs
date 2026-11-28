//! The animation player's loop: a thread per animation decodes its frames (`pg_codec::animation`) and hands each one to a
//! [`Sink`] when its delay is over. Looping forever, as most viewers do in practice: a GIF's loop count isn't honoured.
//! The sink is the caller's (the viewer's video sink, in the desktop); this module knows nothing of it.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pg_codec::animation::{self, Frame};
use pg_model::format::Format;

/// Whether the file's bytes hold an animation: more than one frame, read from the container (`pg_metadata::frames`).
pub fn animated(bytes: &[u8], format: Format) -> bool {
    pg_metadata::frames(bytes, format) > 1
}

/// The bytes of decoded frames an animation keeps for its loops after the first: an animation whose frames fit is decoded
/// once, and a larger one is decoded again at each loop. (A 480 × 270 GIF of 200 frames is about 100 MB, so it fits.)
const FRAME_BUDGET: usize = 128 << 20;

/// Where the frames go. `present` and `failed` may be called from the player's thread. `close` is called once, by the
/// player's `Drop`, and no `present` is called after it (a frame is presented under the lock `Drop` takes to close).
pub trait Sink: Send + Sync + 'static {
    /// A frame of `width` by `height` pixels, as RGBA bytes, to show.
    fn present(&self, width: u32, height: u32, rgba: &[u8]);
    /// The animation could not be played as far as it goes: its file couldn't be read, its frames couldn't be decoded, or
    /// the player's thread couldn't start. `error` says why. What was shown stays, and the player goes on as far as it can.
    fn failed(&self, _error: &str) {}
    /// The player has stopped: nothing more is presented.
    fn close(&self) {}
}

struct Control {
    stop: bool,
    paused: bool,
}

type Shared = Arc<(Mutex<Control>, Condvar)>;

/// The thread playing one animation; dropping it stops it and closes its sink. The thread is not waited for: it ends by
/// itself at its next check of `stop` (a frame's decode is never on the dropping thread).
pub struct Player<S: Sink> {
    control: Shared,
    sink: Arc<S>,
}

impl<S: Sink> Player<S> {
    /// Plays the animation whose bytes `load` gives, into `sink`, from the start, paused or not. `load` runs on the
    /// player's thread (a read, or a wait for a download, never blocks the caller); what it can't give, or the frames it
    /// can't decode, shows nothing and is reported to `sink.failed`.
    pub fn start(
        load: impl FnOnce() -> Result<animation::Bytes, String> + Send + 'static,
        sink: Arc<S>,
        paused: bool,
    ) -> Player<S> {
        Self::start_sized(load, sink, paused, Size::Native)
    }

    /// [`Player::start`], with the frames at `size`: a tile's animation plays at the tile's size, not the file's.
    pub fn start_sized(
        load: impl FnOnce() -> Result<animation::Bytes, String> + Send + 'static,
        sink: Arc<S>,
        paused: bool,
        size: Size,
    ) -> Player<S> {
        let control: Shared = Arc::new((Mutex::new(Control { stop: false, paused }), Condvar::new()));
        let spawned = std::thread::Builder::new().name("animation".into()).spawn({
            let (control, sink) = (control.clone(), sink.clone());
            move || play(load, &*sink, &control, size)
        });
        // The thread is detached: its handle is not kept, and it holds what it plays (`sink`, the bytes) until it ends.
        if let Err(e) = spawned {
            sink.failed(&format!("the animation's thread couldn't start: {e}"));
        }
        Player { control, sink }
    }

    /// Pauses or resumes the playback.
    pub fn set_paused(&self, paused: bool) {
        let (lock, wake) = &*self.control;
        lock.lock().unwrap_or_else(PoisonError::into_inner).paused = paused;
        wake.notify_all();
    }
}

impl<S: Sink> Drop for Player<S> {
    fn drop(&mut self) {
        // Under the lock `present` takes for its check of `stop`: once this returns, no frame reaches the sink.
        let (lock, wake) = &*self.control;
        let mut control = lock.lock().unwrap_or_else(PoisonError::into_inner);
        control.stop = true;
        wake.notify_all();
        self.sink.close();
    }
}

/// A loader that gives the shared bytes `load` gives (a file kept in memory, such as a search's download): no copy.
pub fn shared<T: AsRef<[u8]> + Send + Sync + 'static>(
    load: impl FnOnce() -> Result<Arc<T>, String> + Send + 'static,
) -> impl FnOnce() -> Result<animation::Bytes, String> + Send + 'static {
    move || load().map(animation::Bytes::shared)
}

/// The loader of an animation file at `path`: its bytes, read on the player's thread, or the reason they can't be.
pub fn read_file(path: PathBuf) -> impl FnOnce() -> Result<animation::Bytes, String> + Send + 'static {
    move || match std::fs::read(&path) {
        Ok(bytes) => Ok(animation::Bytes::from(bytes)),
        Err(e) => Err(format!("the file couldn't be read: {e}")),
    }
}

/// Presents one frame if the player is still running: the check of `stop` and the sink's `present` are one critical
/// section, so a frame is never presented after `Drop` has closed the sink. False once stopped.
fn present(control: &Shared, sink: &impl Sink, width: u32, height: u32, rgba: &[u8]) -> bool {
    let (lock, _) = &**control;
    let control = lock.lock().unwrap_or_else(PoisonError::into_inner);
    if control.stop {
        return false;
    }
    sink.present(width, height, rgba);
    true
}

/// The size the frames are shown at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// As the file has them.
    Native,
    /// Scaled to this height in pixels, keeping the aspect ratio, when taller (never enlarged): a tile's size.
    Height(u32),
}

/// The frame at `size`: scaled by the MKS 2021 kernel (premultiplied, so a transparent edge keeps its colour), or as it is.
fn fit(frame: Frame, size: Size, resizer: &mut pg_codec::resize::Resizer) -> pg_codec::Result<Frame> {
    let Size::Height(target) = size else { return Ok(frame) };
    if frame.height <= target || frame.height == 0 {
        return Ok(frame);
    }
    // The width from the ratio, in 64 bits so the product of two sides can't overflow.
    let width = (u64::from(frame.width) * u64::from(target)).div_ceil(u64::from(frame.height)).max(1);
    let (width, source_width, source_height) = (
        usize::try_from(width).unwrap_or(usize::MAX),
        usize::try_from(frame.width).unwrap_or(usize::MAX),
        usize::try_from(frame.height).unwrap_or(usize::MAX),
    );
    let mut rgba = Vec::new();
    resizer.resize_rgba(&frame.rgba, (source_width, source_height), (width, target as usize), &mut rgba)?;
    Ok(Frame { width: u32::try_from(width).unwrap_or(u32::MAX), height: target, rgba, delay_ms: frame.delay_ms })
}

/// Loops over the animation's frames until stopped. Each frame is shown when the previous one's delay is over
/// (decoding the next counts toward it). Bytes that can't be given, or a first frame that fails, show nothing: the
/// viewer keeps the poster.
fn play(load: impl FnOnce() -> Result<animation::Bytes, String>, sink: &impl Sink, control: &Shared, size: Size) {
    let bytes = match load() {
        Ok(bytes) => bytes,
        Err(e) => {
            sink.failed(&e);
            return;
        }
    };
    let mut resizer = pg_codec::resize::Resizer::new();
    let mut due = Instant::now();
    // The frames of the first pass, kept for the loops after it while they fit in `FRAME_BUDGET`; `None` once they don't.
    let mut kept: Option<Vec<Frame>> = Some(Vec::new());
    let mut kept_bytes = 0usize;
    loop {
        if let Some(frames) = kept.as_ref().filter(|frames| !frames.is_empty()) {
            for frame in frames {
                if !wait_until(control, &mut due) {
                    return;
                }
                if !present(control, sink, frame.width, frame.height, &frame.rgba) {
                    return;
                }
                due = next_due(due, frame);
            }
            continue;
        }
        let pass = match animation::frames(bytes.clone()) {
            Ok(pass) => pass,
            Err(e) => {
                sink.failed(&format!("the animation couldn't be decoded: {e}"));
                return;
            }
        };
        let mut shown = false;
        // A frame that fails to decode ends the pass: what came before it still loops, and the failure is said.
        for item in pass {
            let frame = match item.and_then(|frame| fit(frame, size, &mut resizer)) {
                Ok(frame) => frame,
                Err(e) => {
                    sink.failed(&format!("a frame couldn't be decoded: {e}"));
                    break;
                }
            };
            if !wait_until(control, &mut due) {
                return;
            }
            if !present(control, sink, frame.width, frame.height, &frame.rgba) {
                return;
            }
            shown = true;
            due = next_due(due, &frame);
            if kept_bytes + frame.rgba.len() > FRAME_BUDGET {
                kept = None;
            }
            if let Some(frames) = kept.as_mut() {
                kept_bytes += frame.rgba.len();
                frames.push(frame);
            }
        }
        if !shown {
            return;
        }
    }
}

/// When the frame after the one shown at `due` is due: its delay on from `due`, or from now when that is already past (a
/// slow decode, a busy machine), so the playback carries on from now rather than rushing to catch up.
fn next_due(due: Instant, frame: &Frame) -> Instant {
    (due + Duration::from_millis(frame.delay_ms.into())).max(Instant::now())
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

    /// Records the sizes of the frames it is given, the failures it is told of, and whether it was closed.
    #[derive(Default)]
    struct Record {
        presented: Mutex<Vec<(u32, u32)>>,
        failed: Mutex<Vec<String>>,
        closed: Mutex<bool>,
    }

    impl Sink for Record {
        fn present(&self, width: u32, height: u32, _rgba: &[u8]) {
            self.presented.lock().unwrap().push((width, height));
        }
        fn failed(&self, error: &str) {
            self.failed.lock().unwrap().push(error.to_string());
        }
        fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    // Item 6: stopping never waits for the player's own work. A loader that is still running when the player is dropped
    // does not hold the drop up, and nothing is presented after the drop closed the sink.
    #[test]
    fn dropping_does_not_wait_for_the_loader() {
        let sink = Arc::new(Record::default());
        let started = std::time::Instant::now();
        let player = Player::start(
            || {
                std::thread::sleep(Duration::from_millis(1500));
                Ok(animation::Bytes::from(pg_codec::fixtures::gif(3).unwrap()))
            },
            sink.clone(),
            false,
        );
        drop(player);
        assert!(started.elapsed() < Duration::from_millis(1000), "the drop waited for the loader");
        assert!(*sink.closed.lock().unwrap());
    }

    #[test]
    fn no_frame_is_presented_after_the_drop() {
        let sink = Arc::new(Record::default());
        let player =
            Player::start(|| Ok(animation::Bytes::from(pg_codec::fixtures::gif(3).unwrap())), sink.clone(), false);
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show a frame",
            || !sink.presented.lock().unwrap().is_empty(),
        );
        drop(player);
        let at_drop = sink.presented.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(sink.presented.lock().unwrap().len(), at_drop, "a frame came after the drop");
    }

    #[test]
    fn a_loader_that_fails_shows_nothing_and_says_why() {
        let sink = Arc::new(Record::default());
        let player = Player::start(|| Err("the file couldn't be read: gone".to_string()), sink.clone(), false);
        // The loader runs on the player's thread: its failure is reported there, after the start.
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to report the failure",
            || !sink.failed.lock().unwrap().is_empty(),
        );
        drop(player);
        assert!(sink.presented.lock().unwrap().is_empty());
        assert_eq!(*sink.failed.lock().unwrap(), ["the file couldn't be read: gone"]);
        assert!(*sink.closed.lock().unwrap());
    }

    // A tile's animation: frames taller than the height are scaled to it (the width from the ratio), and never enlarged.
    #[test]
    fn frames_are_scaled_to_the_height_and_never_enlarged() {
        let sink = Arc::new(Record::default());
        let player = Player::start_sized(
            || Ok(animation::Bytes::from(pg_codec::fixtures::gif(3).unwrap())),
            sink.clone(),
            false,
            Size::Height(2),
        );
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show its frames",
            || !sink.presented.lock().unwrap().is_empty(),
        );
        drop(player);
        assert!(
            sink.presented.lock().unwrap().iter().all(|&size| size == (2, 2)),
            "{:?}",
            sink.presented.lock().unwrap()
        );

        let sink = Arc::new(Record::default());
        let player = Player::start_sized(
            || Ok(animation::Bytes::from(pg_codec::fixtures::gif(3).unwrap())),
            sink.clone(),
            false,
            Size::Height(9),
        );
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show its frames",
            || !sink.presented.lock().unwrap().is_empty(),
        );
        drop(player);
        assert!(
            sink.presented.lock().unwrap().iter().all(|&size| size == (4, 4)),
            "a frame smaller than the height grew"
        );
    }

    #[test]
    fn plays_bytes_with_no_file() {
        let sink = Arc::new(Record::default());
        let player =
            Player::start(|| Ok(animation::Bytes::from(pg_codec::fixtures::gif(3).unwrap())), sink.clone(), false);
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show its frames",
            || sink.presented.lock().unwrap().len() > 3,
        );
        assert!(sink.presented.lock().unwrap().iter().all(|&size| size == (4, 4)));
        drop(player);
    }

    #[test]
    fn loops_the_frames_and_closes_the_sink_once_dropped() {
        let dir = pg_testkit::run_folder("scratch_render", "player").unwrap();
        let file = dir.join("loop.gif");
        std::fs::write(&file, pg_codec::fixtures::gif(3).unwrap()).unwrap();

        let sink = Arc::new(Record::default());
        let player = Player::start(read_file(file.clone()), sink.clone(), false);
        // Three frames a pass, so seeing four shows the loop started again.
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show its frames",
            || sink.presented.lock().unwrap().len() > 3,
        );
        assert!(sink.presented.lock().unwrap().iter().all(|&size| size == (4, 4)));
        drop(player);
        assert!(*sink.closed.lock().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_shows_nothing() {
        let sink = Arc::new(Record::default());
        let player = Player::start(read_file("no-such-animation.gif".into()), sink.clone(), false);
        drop(player);
        assert!(sink.presented.lock().unwrap().is_empty());
        assert!(*sink.closed.lock().unwrap());
    }

    #[test]
    fn animated_is_more_than_one_frame() {
        assert!(!animated(&pg_codec::fixtures::gif(1).unwrap(), Format::Gif));
        assert!(animated(&pg_codec::fixtures::gif(3).unwrap(), Format::Gif));
        assert!(animated(&pg_codec::fixtures::apng(2).unwrap(), Format::Png));
    }

    // A file whose name has no UTF-8 form (a lone surrogate on Windows, a raw byte on Unix) plays from its exact path.
    #[test]
    fn plays_a_file_whose_name_is_not_utf8() {
        use std::ffi::OsString;
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[
                u16::from(b'x'),
                0xD800,
                u16::from(b'.'),
                u16::from(b'g'),
                u16::from(b'i'),
                u16::from(b'f'),
            ])
        };
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"x\xFF.gif".to_vec())
        };
        let dir = pg_testkit::run_folder("scratch_render", "player-odd").unwrap();
        let file = dir.join(name);
        std::fs::write(&file, pg_codec::fixtures::gif(3).unwrap()).unwrap();

        let sink = Arc::new(Record::default());
        let player = Player::start(read_file(file.clone()), sink.clone(), false);
        pg_testkit::wait_until(
            Duration::from_secs(10),
            || "the player to show its frames",
            || !sink.presented.lock().unwrap().is_empty(),
        );
        assert!(sink.presented.lock().unwrap().iter().all(|&size| size == (4, 4)));
        drop(player);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
