//! What a running pass shares with whoever started it: progress (printed on stderr for the command line, polled
//! by the app), pause, and a stop request (a new job, a new thumbnail height, quitting).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Progress {
    /// "crop", "scan", "import", "borders", "thumbs"; empty when idle.
    pub phase: &'static str,
    /// The folder being scanned or saved into, or the thumbnail height.
    pub detail: String,
    pub done: u64,
    /// 0 when unknown (scanning).
    pub total: u64,
    pub failed: u64,
    pub bytes: u64,
    pub started: Instant,
}

impl Progress {
    fn idle() -> Progress {
        Progress { phase: "", detail: String::new(), done: 0, total: 0, failed: 0, bytes: 0, started: Instant::now() }
    }

    fn rate(&self) -> f64 {
        self.done as f64 / self.started.elapsed().as_secs_f64().max(1e-3)
    }

    fn eta(&self) -> f64 {
        let rate = self.rate();
        if rate > 0.0 { self.total.saturating_sub(self.done) as f64 / rate } else { 0.0 }
    }

    /// The command line's progress line.
    pub fn line(&self) -> String {
        let secs = self.started.elapsed().as_secs_f64().max(1e-3);
        if self.total == 0 {
            return format!(
                "  {} {}: {} files, {:.0} files/s, {secs:.0} s",
                self.phase,
                self.detail,
                self.done,
                self.rate()
            );
        }
        format!(
            "  {}: {}/{} ({:.1}%), {:.0} files/s, {:.0} MB/s, {} failed, {secs:.0} s elapsed, ETA {:.0} s",
            self.phase,
            self.done,
            self.total,
            100.0 * self.fraction(),
            self.rate(),
            self.bytes as f64 / secs / 1e6,
            self.failed,
            self.eta()
        )
    }

    /// The app's status text.
    pub fn status(&self) -> String {
        let n = |v: u64| group_digits(v);
        match self.phase {
            "" => String::new(),
            "scan" => format!("Scanning {} · {} images found", self.detail, n(self.done)),
            phase => {
                let what = match phase {
                    "import" => "Importing".to_string(),
                    "borders" => "Finding borders".to_string(),
                    "crop" => "Saving cropped copies".to_string(),
                    _ => format!("Thumbnails at {}", self.detail),
                };
                let failed = if self.failed > 0 { format!(" · {} failed", n(self.failed)) } else { String::new() };
                format!(
                    "{what} {} of {} · {:.0}/s · {}{failed}",
                    n(self.done),
                    n(self.total),
                    self.rate(),
                    duration(self.eta())
                )
            }
        }
    }

    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.done as f64 / self.total as f64 }
    }
}

fn group_digits(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..60 => format!("{s} s left"),
        60..3600 => format!("{} min left", s.div_ceil(60)),
        _ => format!("{} h {} min left", s / 3600, s % 3600 / 60),
    }
}

pub struct Control {
    stop: AtomicBool,
    paused: AtomicBool,
    /// Print progress lines on stderr (the command line).
    print: bool,
    progress: Mutex<(Progress, Instant)>,
}

impl Control {
    pub fn new(print: bool) -> Control {
        Control {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            print,
            progress: Mutex::new((Progress::idle(), Instant::now())),
        }
    }

    /// Ask the running pass to stop at the next file; it commits what it has.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn clear_stop(&self) {
        self.stop.store(false, Ordering::Relaxed);
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Called before each unit of work: waits while paused; false means stop.
    pub fn checkpoint(&self) -> bool {
        loop {
            if self.stop_requested() {
                return false;
            }
            if !self.is_paused() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn begin(&self, phase: &'static str, detail: String, total: u64) {
        let now = Instant::now();
        *self.progress.lock().unwrap() =
            (Progress { phase, detail, done: 0, total, failed: 0, bytes: 0, started: now }, now);
    }

    /// One file done.
    pub fn add(&self, bytes: usize, failed: bool) {
        self.update(|p| {
            p.done += 1;
            p.failed += failed as u64;
            p.bytes += bytes as u64;
        });
    }

    /// Set the count directly (the scanner counts images found).
    pub fn set_done(&self, done: u64) {
        self.update(|p| p.done = done);
    }

    fn update(&self, f: impl FnOnce(&mut Progress)) {
        let mut guard = self.progress.lock().unwrap();
        let (p, last) = &mut *guard;
        f(p);
        if self.print && last.elapsed() >= Duration::from_secs(5) {
            *last = Instant::now();
            eprintln!("{}", p.line());
        }
    }

    /// The phase ended: print its final line, then go idle.
    pub fn finish(&self) {
        let mut guard = self.progress.lock().unwrap();
        if self.print {
            eprintln!("{}", guard.0.line());
        }
        guard.0 = Progress::idle();
    }

    pub fn snapshot(&self) -> Progress {
        self.progress.lock().unwrap().0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(duration(5.2), "5 s left");
        assert_eq!(duration(61.0), "2 min left");
        assert_eq!(duration(7300.0), "2 h 1 min left");
    }
}
