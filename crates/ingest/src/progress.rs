//! Periodic progress lines on stderr: count, rate, throughput, failures, ETA.

use std::time::{Duration, Instant};

pub struct Progress {
    label: &'static str,
    total: usize,
    done: usize,
    failed: usize,
    bytes: u64,
    start: Instant,
    last: Instant,
}

impl Progress {
    pub fn new(label: &'static str, total: usize) -> Progress {
        let now = Instant::now();
        Progress { label, total, done: 0, failed: 0, bytes: 0, start: now, last: now }
    }

    pub fn add(&mut self, bytes: usize, failed: bool) {
        self.done += 1;
        self.failed += failed as usize;
        self.bytes += bytes as u64;
        if self.last.elapsed() >= Duration::from_secs(5) {
            self.last = Instant::now();
            self.print();
        }
    }

    pub fn finish(&self) {
        self.print();
    }

    fn print(&self) {
        let secs = self.start.elapsed().as_secs_f64().max(1e-3);
        let rate = self.done as f64 / secs;
        let eta = if rate > 0.0 { (self.total - self.done) as f64 / rate } else { 0.0 };
        eprintln!(
            "  {}: {}/{} ({:.1}%), {:.0} files/s, {:.0} MB/s, {} failed, {:.0} s elapsed, ETA {:.0} s",
            self.label,
            self.done,
            self.total,
            100.0 * self.done as f64 / self.total.max(1) as f64,
            rate,
            self.bytes as f64 / secs / 1e6,
            self.failed,
            secs,
            eta
        );
    }
}
