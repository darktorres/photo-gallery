//! The reader-count controller: how many reads a lane's pass keeps in flight on its drive. A pure, deterministic state
//! machine: a [`Sample`] of one window of a pass goes in, a [`Decision`] comes out. It has no thread and no clock; time
//! is the windows' own `elapsed`, summed. The pipeline's sampler (`pipeline::sample`) feeds it and moves the reader gate
//! to its target.
//!
//! **The algorithm is a candidate, chosen before the measurements** (`docs/plans/ADAPTIVE_READERS_PLAN.md`): a hill
//! climb with a latency guard. Its thresholds are the named, provisional constants of [`TunerConfig`]. Only the
//! interface is fixed: a sample in, a decision out.
//!
//! - **Windows.** A window is judged only when it is long enough, has enough reads, had no pause, and is not the first
//!   after a change of count (its reads in flight were the old count's). A window whose readers wait on the workers or
//!   whose workers wait on the CPU (`blocked`) is decode-bound: the count does not matter there, so it holds and teaches
//!   nothing. Three judged windows at one count make a decision, on their median.
//! - **Explore.** From the start (a stored profile, or the drive kind's prior), the count is halved while the lower one
//!   is as good (within [`TunerConfig::keep_lower`]), then doubled while the higher one gains at least
//!   [`TunerConfig::grow_gain`], then refined by one step down and up by the same rules. Fewer readers win a tie: they
//!   hold less memory and each read waits less.
//! - **Stable.** The latency guard backs off a step when the reads take more than [`TunerConfig::latency_ratio`] times
//!   the lowest count's for a gain under [`TunerConfig::latency_gain`]; a collapse below [`TunerConfig::collapse`] of the
//!   next lower count's throughput backs off and holds the count down (30 s, doubling to 5 min). A read in flight older
//!   than [`TunerConfig::stall`] backs off at once, before it completes. Every [`TunerConfig::probe_every`] judged windows
//!   one step is probed (down, or up from one) and kept or reverted by the same rules; a kept probe explores on from it,
//!   which is how a cache that warms or cools is followed.
//! - **Noise.** A count whose last windows vary by more than [`TunerConfig::noise_cv`] (their coefficient of variation)
//!   holds and probes nothing.

use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroUsize;
use std::time::Duration;

use pg_model::num::{count, size};
use pg_platform::drive::Kind;

/// The most reads in flight a tuned pass keeps: the bound of its readers, its slots and its budget.
pub const MAX_READERS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(15);

/// One window of a pass, as the pipeline's meters saw it. Times are summed over the threads of their stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// The window's wall time.
    pub elapsed: Duration,
    /// How long the lane was paused in it (the difference of `Control::paused_total`): any pause discards the window.
    pub paused: Duration,
    /// The reads that completed in it.
    pub reads: u64,
    /// The time those reads spent at the disk: around the read only, not the pool's wait.
    pub disk_ns: u64,
    /// The bytes those reads gave.
    pub bytes: u64,
    /// The age of the oldest read still in flight at the window's end (0: none).
    pub oldest_read_ns: u64,
    /// The workers' wait for a read file.
    pub work_wait_ns: u64,
    /// The workers' wait for a CPU permit (the lanes share the cores).
    pub permit_wait_ns: u64,
    /// The workers' processing.
    pub work_busy_ns: u64,
    /// The readers' wait for their item's turn in the pool's in-order grants.
    pub pool_order_wait_ns: u64,
    /// The readers' wait for a buffer or a slot: the workers are behind.
    pub pool_capacity_wait_ns: u64,
    /// The readers let through by the gate, on average over the window.
    pub active_readers: f64,
    /// The pass's workers.
    pub workers: usize,
    /// The files the readers handed on.
    pub files: u64,
}

impl Sample {
    /// The share of the workers' time they waited for reads: the pass is read-bound when it is high.
    pub fn starved(&self) -> f64 {
        ratio(size(self.work_wait_ns), count(self.workers.max(1)) * nanos(self.elapsed))
    }

    /// The share of the readers' and workers' time they waited on each other or on the CPU: the pass is decode-bound
    /// when it is high.
    pub fn blocked(&self) -> f64 {
        let threads = self.active_readers.max(0.0) + count(self.workers.max(1));
        ratio(size(self.pool_capacity_wait_ns.saturating_add(self.permit_wait_ns)), threads * nanos(self.elapsed))
    }

    /// The bytes per second the window read.
    pub fn throughput(&self) -> f64 {
        ratio(size(self.bytes), self.elapsed.as_secs_f64())
    }

    /// The mean time a read spent at the disk, in nanoseconds (0 with no read).
    pub fn latency(&self) -> f64 {
        ratio(size(self.disk_ns), size(self.reads))
    }
}

/// `a / b`, or 0 when `b` is not positive.
fn ratio(a: f64, b: f64) -> f64 {
    if b > 0.0 { a / b } else { 0.0 }
}

/// A duration in nanoseconds, as a float.
fn nanos(d: Duration) -> f64 {
    d.as_secs_f64() * 1e9
}

/// Why the target is what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The first count: the prior's or the profile's.
    Start,
    /// The window was not judged (too short, too few reads, a pause, or the first after a change).
    Discarded,
    /// Collecting windows at this count.
    Measuring,
    /// The workers or the CPU are the limit, not the reads: the count is held.
    DecodeBound,
    /// The windows vary too much to compare: the count is held.
    Noisy,
    /// Exploring: a count to measure.
    Explore,
    /// Explored: the best count found.
    Settled,
    /// Stable at the best count.
    Hold,
    /// The reads took much longer for little gain: one step down.
    LatencyBackoff,
    /// The throughput fell well below the next lower count's: down, and held down for a while.
    Collapse,
    /// A read in flight stalled: one step down, and held down.
    Stall,
    /// A step probed from the stable count.
    Probe,
    /// The probed step was as good, or better: kept, and explored on from.
    ProbeKept,
    /// The probed step was worse: back to the stable count.
    ProbeReverted,
    /// The stored profile's throughput was not reached: started again from the drive kind's prior.
    Distrusted,
}

impl Reason {
    /// The reason's name, as the perf log and the dev-mode log show it.
    pub fn name(self) -> &'static str {
        match self {
            Reason::Start => "start",
            Reason::Discarded => "discarded",
            Reason::Measuring => "measuring",
            Reason::DecodeBound => "decode-bound",
            Reason::Noisy => "noisy",
            Reason::Explore => "explore",
            Reason::Settled => "settled",
            Reason::Hold => "hold",
            Reason::LatencyBackoff => "latency-backoff",
            Reason::Collapse => "collapse",
            Reason::Stall => "stall",
            Reason::Probe => "probe",
            Reason::ProbeKept => "probe-kept",
            Reason::ProbeReverted => "probe-reverted",
            Reason::Distrusted => "distrusted",
        }
    }
}

/// What the tuner decided after a sample: the readers to let through, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The readers to let through.
    pub target: NonZeroUsize,
    /// Why.
    pub reason: Reason,
}

/// The tuner's thresholds. Provisional: chosen before the measurements that are to lock them
/// (`docs/plans/ADAPTIVE_READERS_PLAN.md`, phase C5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TunerConfig {
    /// The most readers.
    pub max: NonZeroUsize,
    /// How often the sampler takes a window.
    pub window: Duration,
    /// The shortest window that is judged.
    pub min_window: Duration,
    /// The fewest reads a judged window has.
    pub min_reads: u64,
    /// The judged windows a decision takes (their median).
    pub judge_windows: usize,
    /// A higher count is kept when it reads at least this much more (0.12: above the ±10% run noise of one HDD).
    pub grow_gain: f64,
    /// A lower count is kept when it reads at least this share of the higher one's.
    pub keep_lower: f64,
    /// The latency guard: reads this many times slower than at the lowest count...
    pub latency_ratio: f64,
    /// ...for less than this gain over the next lower count back off one step.
    pub latency_gain: f64,
    /// A count reading less than this share of the next lower count's throughput collapses to it.
    pub collapse: f64,
    /// The first hold-down after a collapse or a stall; each next one doubles, up to `hold_down_max`.
    pub hold_down: Duration,
    /// The longest hold-down.
    pub hold_down_max: Duration,
    /// A read in flight this old is a stall.
    pub stall: Duration,
    /// Judged windows at a stable count between probes.
    pub probe_every: u32,
    /// The windows whose variation is the noise test.
    pub noise_windows: usize,
    /// The coefficient of variation above which the windows are noise.
    pub noise_cv: f64,
    /// A window whose `blocked` share is at least this is decode-bound.
    pub blocked_max: f64,
    /// A window whose `starved` share is at least this is read-bound (what a profile is learned from).
    pub starved_min: f64,
    /// A measured count older than this is measured again before it is compared.
    pub stale_after: Duration,
    /// How much of a count's stored throughput a new median replaces.
    pub decay: f64,
    /// A stored profile is distrusted when the first judged median is under this share of its throughput.
    pub distrust: f64,
}

impl Default for TunerConfig {
    fn default() -> TunerConfig {
        TunerConfig {
            max: MAX_READERS,
            window: Duration::from_secs(3),
            min_window: Duration::from_millis(1500),
            min_reads: 16,
            judge_windows: 3,
            grow_gain: 0.12,
            keep_lower: 0.95,
            latency_ratio: 2.0,
            latency_gain: 0.05,
            collapse: 0.88,
            hold_down: Duration::from_secs(30),
            hold_down_max: Duration::from_secs(300),
            stall: Duration::from_secs(2),
            probe_every: 20,
            noise_windows: 5,
            noise_cv: 0.35,
            blocked_max: 0.5,
            starved_min: 0.1,
            stale_after: Duration::from_secs(180),
            decay: 0.5,
            distrust: 0.7,
        }
    }
}

impl TunerConfig {
    /// The default thresholds with windows of `window`, the shortest judged one half of it: for a test whose time is
    /// scaled down. The hold-downs and staleness scale with it (as 10 and 100 and 60 windows).
    pub fn scaled(window: Duration) -> TunerConfig {
        TunerConfig {
            window,
            min_window: window / 2,
            hold_down: window * 10,
            hold_down_max: window * 100,
            stall: window * 2 / 3,
            stale_after: window * 60,
            ..TunerConfig::default()
        }
    }
}

/// Where a tuner starts: a count, and when it comes from a stored profile, the throughput the profile measured there
/// (bytes per second), which the first judged median is checked against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prior {
    /// The first count.
    pub start: NonZeroUsize,
    /// The profile's throughput at `start`, when the start is a profile's.
    pub expected: Option<f64>,
    /// The count to start again from when the profile is distrusted: the drive kind's.
    pub fallback: NonZeroUsize,
}

impl Prior {
    /// The prior of a drive kind, with no profile: 2 readers on a spinning disk, 4 on a flash disk, 3 when unknown.
    pub fn of_kind(kind: Kind) -> Prior {
        let start = NonZeroUsize::MIN.saturating_add(match kind {
            Kind::Rotational => 1,
            Kind::Solid => 3,
            Kind::Unknown => 2,
        });
        Prior { start, expected: None, fallback: start }
    }

    /// A stored profile's count and throughput, on a drive of `kind` (the fallback).
    pub fn of_profile(readers: NonZeroUsize, bytes_per_sec: f64, kind: Kind) -> Prior {
        Prior { start: readers, expected: Some(bytes_per_sec), fallback: Prior::of_kind(kind).start }
    }
}

/// What a count read when it was last measured.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stat {
    /// Bytes per second (a decayed median).
    tput: f64,
    /// Mean read latency, in nanoseconds.
    latency: f64,
    /// When it was measured (the tuner's clock).
    at: Duration,
}

/// The direction and step of an exploration phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    DownDouble,
    UpDouble,
    DownOne,
    UpOne,
}

impl Phase {
    /// The phase after this one: halve, double, one down, one up, then none.
    fn after(self) -> Option<Phase> {
        match self {
            Phase::DownDouble => Some(Phase::UpDouble),
            Phase::UpDouble => Some(Phase::DownOne),
            Phase::DownOne => Some(Phase::UpOne),
            Phase::UpOne => None,
        }
    }

    /// The count after `from` in this phase's direction and step, if it is within `1..=max`.
    fn step(self, from: usize, max: usize) -> Option<usize> {
        let next = match self {
            Phase::DownDouble => from / 2,
            Phase::UpDouble => from.saturating_mul(2).min(max),
            Phase::DownOne => from.saturating_sub(1),
            Phase::UpOne => from.saturating_add(1),
        };
        (next >= 1 && next <= max && next != from).then_some(next)
    }
}

/// What the tuner is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Exploring in `phase`, the best count so far `best`.
    Explore { phase: Phase, best: usize },
    /// At the best count, `windows` judged windows since the last probe.
    Stable { windows: u32 },
    /// Probing `to` from the stable count `from`.
    Probe { from: usize },
}

/// What a tuner learned, for its profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Learned {
    /// The count it holds.
    pub readers: NonZeroUsize,
    /// The lowest and highest counts it measured.
    pub tested: (usize, usize),
    /// The throughput at `readers`, bytes per second (0 when not measured).
    pub bytes_per_sec: f64,
    /// The time and the reads of its read-bound windows.
    pub read_bound: Duration,
    /// The reads of its read-bound windows.
    pub read_bound_reads: u64,
    /// Whether its last decision was a stable one.
    pub stable: bool,
}

/// The controller of one pass kind on one drive. See the module comment.
#[derive(Clone, Debug)]
pub struct Tuner {
    cfg: TunerConfig,
    prior: Prior,
    target: usize,
    mode: Mode,
    /// What each count read, when last measured.
    table: BTreeMap<usize, Stat>,
    /// The judged windows at the current count since the last decision: throughput and latency.
    judged: Vec<(f64, f64)>,
    /// The last throughputs at the current count, for the noise test.
    recent: VecDeque<f64>,
    /// The next window spans the last change: it is not judged.
    skip_next: bool,
    /// The sum of every window's time: the tuner's clock.
    clock: Duration,
    /// No count above the current one until then.
    held_until: Duration,
    /// The next hold-down's length.
    hold_down: Duration,
    /// Whether the profile was checked against the first judged median.
    checked: bool,
    /// The read-bound windows' time and reads.
    read_bound: Duration,
    read_bound_reads: u64,
    /// The last decision's reason.
    last: Reason,
    /// The changes of count, and the probes.
    changes: u64,
    probes: u64,
}

impl Tuner {
    /// A tuner of `cfg`, starting at `prior` (clamped into `1..=cfg.max`).
    pub fn new(cfg: TunerConfig, prior: Prior) -> Tuner {
        let target = prior.start.get().min(cfg.max.get());
        Tuner {
            cfg,
            prior,
            target,
            mode: Mode::Explore { phase: Phase::DownDouble, best: target },
            table: BTreeMap::new(),
            judged: Vec::new(),
            recent: VecDeque::new(),
            skip_next: false,
            clock: Duration::ZERO,
            held_until: Duration::ZERO,
            hold_down: cfg.hold_down,
            checked: false,
            read_bound: Duration::ZERO,
            read_bound_reads: 0,
            last: Reason::Start,
            changes: 0,
            probes: 0,
        }
    }

    /// The thresholds.
    pub fn config(&self) -> &TunerConfig {
        &self.cfg
    }

    /// The readers to let through now.
    pub fn target(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.target).unwrap_or(NonZeroUsize::MIN)
    }

    /// The last decision's reason.
    pub fn reason(&self) -> Reason {
        self.last
    }

    /// The changes of count so far.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The probes so far.
    pub fn probes(&self) -> u64 {
        self.probes
    }

    /// What it learned, for its profile.
    pub fn learned(&self) -> Learned {
        let tested = (
            self.table.keys().next().copied().unwrap_or(self.target),
            self.table.keys().next_back().copied().unwrap_or(self.target),
        );
        Learned {
            readers: self.target(),
            tested,
            bytes_per_sec: self.table.get(&self.target).map_or(0.0, |stat| stat.tput),
            read_bound: self.read_bound,
            read_bound_reads: self.read_bound_reads,
            stable: matches!(self.mode, Mode::Stable { .. }) && !matches!(self.last, Reason::Noisy),
        }
    }

    /// One window in, the decision out.
    pub fn update(&mut self, s: &Sample) -> Decision {
        self.clock = self.clock.saturating_add(s.elapsed);
        let reason = self.decide(s);
        self.last = reason;
        Decision { target: self.target(), reason }
    }

    fn decide(&mut self, s: &Sample) -> Reason {
        // A read in flight that stalled is seen before it completes, whatever the window is.
        let stalled = s.oldest_read_ns >= u64::try_from(self.cfg.stall.as_nanos()).unwrap_or(u64::MAX);
        if stalled && self.target > 1 && s.paused.is_zero() {
            self.hold_down_now();
            let lower = self.target - 1;
            self.mode = Mode::Stable { windows: 0 };
            self.move_to(lower);
            return Reason::Stall;
        }
        if std::mem::take(&mut self.skip_next)
            || !s.paused.is_zero()
            || s.elapsed < self.cfg.min_window
            || s.reads < self.cfg.min_reads
        {
            return Reason::Discarded;
        }
        if s.blocked() >= self.cfg.blocked_max {
            return Reason::DecodeBound;
        }
        let (tput, latency) = (s.throughput(), s.latency());
        if s.starved() >= self.cfg.starved_min {
            self.read_bound = self.read_bound.saturating_add(s.elapsed);
            self.read_bound_reads = self.read_bound_reads.saturating_add(s.reads);
        }
        self.recent.push_back(tput);
        while self.recent.len() > self.cfg.noise_windows {
            self.recent.pop_front();
        }
        if self.recent.len() >= self.cfg.noise_windows && variation(&self.recent) > self.cfg.noise_cv {
            self.judged.clear();
            return Reason::Noisy;
        }
        self.judged.push((tput, latency));
        if self.judged.len() < self.cfg.judge_windows {
            return Reason::Measuring;
        }
        let (median, median_latency) = medians(&self.judged);
        self.judged.clear();
        if !std::mem::replace(&mut self.checked, true)
            && let Some(expected) = self.prior.expected
            && median < expected * self.cfg.distrust
        {
            self.table.clear();
            let start = self.prior.fallback.get().min(self.cfg.max.get());
            self.mode = Mode::Explore { phase: Phase::DownDouble, best: start };
            self.move_to(start);
            return Reason::Distrusted;
        }
        self.record(median, median_latency);
        match self.mode {
            Mode::Explore { phase, best } => self.explore(phase, best),
            Mode::Stable { windows } => self.stable(windows, median, median_latency),
            Mode::Probe { from } => self.judge_probe(from, median),
        }
    }

    /// Stores the median measured at the current count, decayed into what was stored.
    fn record(&mut self, tput: f64, latency: f64) {
        let fresh = self.fresh(self.target);
        let decay = self.cfg.decay;
        let entry = self.table.entry(self.target).or_insert(Stat { tput, latency, at: self.clock });
        if fresh {
            entry.tput = entry.tput * (1.0 - decay) + tput * decay;
            entry.latency = entry.latency * (1.0 - decay) + latency * decay;
        } else {
            entry.tput = tput;
            entry.latency = latency;
        }
        entry.at = self.clock;
    }

    /// The count's measurement, when it is recent enough to compare.
    fn stat(&self, readers: usize) -> Option<Stat> {
        self.table.get(&readers).copied().filter(|stat| self.clock.saturating_sub(stat.at) <= self.cfg.stale_after)
    }

    fn fresh(&self, readers: usize) -> bool {
        self.stat(readers).is_some()
    }

    /// Whether `lower` reads as well as `higher`, near enough for the fewer readers to win.
    fn as_good(&self, lower: f64, higher: f64) -> bool {
        lower >= higher * self.cfg.keep_lower
    }

    /// Whether `higher` reads enough more than `lower` to take the more readers.
    fn gains(&self, higher: f64, lower: f64) -> bool {
        higher >= lower * (1.0 + self.cfg.grow_gain)
    }

    /// Whether a count above the current one may be tried now (not held down).
    fn may_grow(&self) -> bool {
        self.clock >= self.held_until
    }

    /// Exploration's next step, after the current count was measured: compares it with the best so far, then walks the
    /// phases until a count needs measuring, or none does and the best is the stable count.
    fn explore(&mut self, mut phase: Phase, mut best: usize) -> Reason {
        let max = self.cfg.max.get();
        // The count just measured, against the best before it.
        let current = self.target;
        if current != best {
            let won = match (self.stat(current), self.stat(best)) {
                (Some(c), Some(b)) if current < best => self.as_good(c.tput, b.tput),
                (Some(c), Some(b)) => self.gains(c.tput, b.tput),
                _ => false,
            };
            if won {
                best = current;
            } else {
                // This direction is spent: on to the next phase, from the best.
                match phase.after() {
                    Some(next) => phase = next,
                    None => return self.settle(best),
                }
            }
        }
        loop {
            // While held down, no count above the best is tried.
            let candidate = phase.step(best, max).filter(|&c| c < best || self.may_grow());
            let Some(candidate) = candidate else {
                match phase.after() {
                    Some(next) => {
                        phase = next;
                        continue;
                    }
                    None => return self.settle(best),
                }
            };
            // A count measured recently is compared now, without moving to it.
            if let (Some(c), Some(b)) = (self.stat(candidate), self.stat(best)) {
                let won = if candidate < best { self.as_good(c.tput, b.tput) } else { self.gains(c.tput, b.tput) };
                if won {
                    best = candidate;
                    continue;
                }
                match phase.after() {
                    Some(next) => {
                        phase = next;
                        continue;
                    }
                    None => return self.settle(best),
                }
            }
            self.mode = Mode::Explore { phase, best };
            self.move_to(candidate);
            return Reason::Explore;
        }
    }

    /// The exploration's end: the best count, held.
    fn settle(&mut self, best: usize) -> Reason {
        self.mode = Mode::Stable { windows: 0 };
        self.move_to(best);
        Reason::Settled
    }

    /// The checks at the stable count, and its probes.
    fn stable(&mut self, windows: u32, median: f64, latency: f64) -> Reason {
        let current = self.target;
        let lower = self.table.range(..current).next_back().map(|(&count, _)| count).filter(|&c| self.fresh(c));
        if let Some(lower) = lower
            && let Some(below) = self.stat(lower)
        {
            if median < below.tput * self.cfg.collapse {
                self.hold_down_now();
                self.mode = Mode::Stable { windows: 0 };
                self.move_to(lower);
                return Reason::Collapse;
            }
            let baseline =
                self.table.iter().find(|&(&count, _)| self.fresh(count)).map_or(latency, |(_, stat)| stat.latency);
            if baseline > 0.0
                && latency > baseline * self.cfg.latency_ratio
                && median < below.tput * (1.0 + self.cfg.latency_gain)
            {
                self.mode = Mode::Stable { windows: 0 };
                self.move_to(current - 1);
                return Reason::LatencyBackoff;
            }
        }
        let windows = windows.saturating_add(u32::try_from(self.cfg.judge_windows).unwrap_or(u32::MAX));
        if windows < self.cfg.probe_every {
            self.mode = Mode::Stable { windows };
            return Reason::Hold;
        }
        let max = self.cfg.max.get();
        let to =
            if current > 1 { Some(current - 1) } else { (current < max && self.may_grow()).then_some(current + 1) };
        let Some(to) = to else {
            self.mode = Mode::Stable { windows: 0 };
            return Reason::Hold;
        };
        self.probes += 1;
        self.mode = Mode::Probe { from: current };
        self.move_to(to);
        Reason::Probe
    }

    /// A probe's verdict, from its median against the stable count's.
    fn judge_probe(&mut self, from: usize, median: f64) -> Reason {
        let probed = self.target;
        let kept = match self.stat(from) {
            Some(stable) if probed < from => self.as_good(median, stable.tput),
            Some(stable) => self.gains(median, stable.tput),
            None => false,
        };
        if kept {
            // Explore on from the probed count, in its direction first.
            let phase = if probed < from { Phase::DownDouble } else { Phase::UpDouble };
            self.mode = Mode::Explore { phase, best: probed };
            let reason = self.explore(phase, probed);
            return if reason == Reason::Settled { Reason::ProbeKept } else { reason };
        }
        self.mode = Mode::Stable { windows: 0 };
        self.move_to(from);
        Reason::ProbeReverted
    }

    /// Holds the counts above the current one down, for the next hold-down, which doubles.
    fn hold_down_now(&mut self) {
        self.held_until = self.clock.saturating_add(self.hold_down);
        self.hold_down = self.hold_down.saturating_mul(2).min(self.cfg.hold_down_max);
    }

    /// Moves the target to `readers`: the windows at the old count are forgotten, and the next window is not judged.
    fn move_to(&mut self, readers: usize) {
        let readers = readers.clamp(1, self.cfg.max.get());
        if readers != self.target {
            self.target = readers;
            self.changes += 1;
            self.skip_next = true;
            self.judged.clear();
            self.recent.clear();
        }
    }
}

/// The medians of the throughputs and of the latencies.
fn medians(windows: &[(f64, f64)]) -> (f64, f64) {
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    (median(windows.iter().map(|w| w.0).collect()), median(windows.iter().map(|w| w.1).collect()))
}

/// The coefficient of variation of `values` (their standard deviation over their mean; 0 for a mean of 0).
fn variation(values: &VecDeque<f64>) -> f64 {
    let n = count(values.len());
    if n == 0.0 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    ratio(var.sqrt(), mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window of 3 s at `mb_s` MB/s with reads of 24 MB that each took `latency_ms`, the workers waiting for reads
    /// half the time (read-bound).
    fn window(mb_s: f64, latency_ms: f64) -> Sample {
        let elapsed = Duration::from_secs(3);
        let bytes = (mb_s * 3e6).max(0.0);
        // 24 MB per read; at least the minimum reads, so every window is judged.
        let reads = 20u64;
        Sample {
            elapsed,
            reads,
            bytes: bytes_of(bytes),
            disk_ns: bytes_of(latency_ms * 1e6 * 20.0),
            work_wait_ns: 6_000_000_000,
            workers: 4,
            active_readers: 2.0,
            files: reads,
            ..Sample::default()
        }
    }

    /// A positive float as whole units (the tests' own conversion, through its decimal text).
    fn bytes_of(x: f64) -> u64 {
        format!("{:.0}", x.max(0.0)).parse().unwrap_or(0)
    }

    /// Runs `tuner` on windows whose throughput (MB/s) and latency (ms) the curve gives at the tuner's target, for
    /// `windows` windows. The decisions.
    fn drive(tuner: &mut Tuner, windows: usize, mut curve: impl FnMut(usize) -> (f64, f64)) -> Vec<Decision> {
        (0..windows)
            .map(|_| {
                let (mb, ms) = curve(tuner.target().get());
                tuner.update(&window(mb, ms))
            })
            .collect()
    }

    fn tuner(start: usize) -> Tuner {
        let start = NonZeroUsize::new(start).unwrap();
        Tuner::new(TunerConfig::default(), Prior { start, expected: None, fallback: start })
    }

    /// The Exos measured whole-file reads: 178, 102, 131 and 115 MB/s at 1, 2, 4 and 8 readers (between them, the
    /// nearer measured count's), latency growing with the count.
    fn exos(readers: usize) -> (f64, f64) {
        let mb = match readers {
            1 => 178.0,
            2 | 3 => 102.0,
            4..=6 => 131.0,
            _ => 115.0,
        };
        (mb, 135.0 * f64::from(u32::try_from(readers).unwrap()))
    }

    /// A flash disk: throughput saturating towards 400 MB/s, latency flat.
    fn ssd(readers: usize) -> (f64, f64) {
        let r = f64::from(u32::try_from(readers).unwrap());
        (400.0 * (1.0 - (-r / 3.0).exp()), 2.0)
    }

    #[test]
    fn a_spinning_disk_that_reads_best_alone_converges_to_one_reader() {
        let mut t = tuner(2);
        drive(&mut t, 60, exos);
        assert!(t.target().get() <= 2, "within a step of one reader: {}", t.target());
    }

    #[test]
    fn a_flash_disk_climbs_to_near_its_saturation() {
        let mut t = tuner(4);
        drive(&mut t, 80, ssd);
        let at = t.target().get();
        assert!((4..=9).contains(&at), "{at}");
        assert!(ssd(at).0 >= 0.85 * 400.0, "{at} readers read {} MB/s", ssd(at).0);
    }

    #[test]
    fn a_clear_gain_grows_the_count() {
        let mut t = tuner(1);
        // Each doubling gains 40%.
        let decisions = drive(&mut t, 12, |r| (100.0 * 1.4f64.powf(f64::from(u32::try_from(r).unwrap()).log2()), 5.0));
        assert!(decisions.iter().any(|d| d.reason == Reason::Explore && d.target.get() > 1));
        assert!(t.target().get() >= 4, "{}", t.target());
    }

    #[test]
    fn no_growth_within_the_noise() {
        let mut t = tuner(2);
        // Flat within 5%: neither more nor fewer readers win; fewer win the tie.
        drive(&mut t, 40, |r| (100.0 + f64::from(u32::try_from(r % 2).unwrap()) * 4.0, 5.0));
        assert!(t.target().get() <= 2, "{}", t.target());
    }

    #[test]
    fn little_gain_at_a_much_higher_latency_keeps_the_lower_count() {
        let mut t = tuner(1);
        // From one to two readers: 3% more, at four times the latency. Two never wins (gain under 12%). Twenty windows
        // settle before the first probe.
        drive(&mut t, 20, |r| if r == 1 { (100.0, 10.0) } else { (103.0, 40.0) });
        assert_eq!(t.target().get(), 1);
    }

    #[test]
    fn a_collapse_backs_off_and_holds_down() {
        let mut t = tuner(1);
        let up_to_four = |r: usize| (50.0 * f64::from(u32::try_from(r.min(4)).unwrap()), 5.0);
        drive(&mut t, 30, up_to_four);
        assert_eq!(t.target().get(), 4, "climbed to four");
        // The disk starts thrashing at four readers.
        let decisions = drive(&mut t, 12, |r| if r >= 4 { (60.0, 5.0) } else { up_to_four(r) });
        assert!(decisions.iter().any(|d| d.reason == Reason::Collapse), "{decisions:?}");
        assert!(t.target().get() < 4);
        assert!(t.clock < t.held_until, "held down after the collapse");
    }

    #[test]
    fn the_latency_guard_backs_off_when_a_gain_fades_at_a_much_higher_latency() {
        let mut t = tuner(2);
        // Two readers first gain 15% over one, at four times the latency: two is kept.
        drive(&mut t, 20, |r| if r == 1 { (100.0, 10.0) } else { (115.0, 40.0) });
        assert_eq!(t.target().get(), 2);
        // The gain fades to 3%, the latency stays: back to one.
        let decisions = drive(&mut t, 8, |r| if r == 1 { (100.0, 10.0) } else { (103.0, 40.0) });
        assert!(decisions.iter().any(|d| d.reason == Reason::LatencyBackoff), "{decisions:?}");
        assert_eq!(t.target().get(), 1);
    }

    #[test]
    fn a_stall_backs_off_before_the_read_completes() {
        let mut t = tuner(4);
        let stalled = Sample { oldest_read_ns: 5_000_000_000, reads: 0, ..window(0.0, 0.0) };
        let decision = t.update(&stalled);
        assert_eq!((decision.reason, decision.target.get()), (Reason::Stall, 3));
    }

    #[test]
    fn a_decode_bound_window_holds_and_teaches_nothing() {
        let mut t = tuner(3);
        let blocked = Sample { pool_capacity_wait_ns: 20_000_000_000, ..window(100.0, 5.0) };
        for _ in 0..10 {
            assert_eq!(t.update(&blocked), Decision { target: t.target(), reason: Reason::DecodeBound });
        }
        assert!(t.table.is_empty(), "nothing learned");
        assert_eq!(t.target().get(), 3);
    }

    #[test]
    fn short_paused_and_thin_windows_are_discarded() {
        let mut t = tuner(3);
        let short = Sample { elapsed: Duration::from_millis(100), ..window(100.0, 5.0) };
        let paused = Sample { paused: Duration::from_millis(1), ..window(100.0, 5.0) };
        let thin = Sample { reads: 3, ..window(100.0, 5.0) };
        for s in [short, paused, thin] {
            assert_eq!(t.update(&s).reason, Reason::Discarded);
        }
        assert!(t.judged.is_empty());
    }

    #[test]
    fn the_window_after_a_change_is_discarded() {
        let mut t = tuner(2);
        let decisions = drive(&mut t, 4, |_| (100.0, 5.0));
        let changed = decisions.iter().position(|d| d.reason == Reason::Explore).expect("a change");
        assert_eq!(decisions.get(changed + 1).map(|d| d.reason), Some(Reason::Discarded));
    }

    #[test]
    fn a_stable_count_is_probed_and_a_worse_probe_reverted() {
        let mut t = tuner(4);
        // Four is clearly best: two reads half as fast, eight no faster.
        let curve = |r: usize| match r {
            1..=3 => (60.0, 5.0),
            4..=7 => (120.0, 5.0),
            _ => (121.0, 5.0),
        };
        let decisions = drive(&mut t, 120, curve);
        assert!(decisions.iter().any(|d| d.reason == Reason::Probe));
        // Each probe of three is reverted to four (the run may end in the middle of the next probe).
        let reverted: Vec<usize> =
            decisions.iter().filter(|d| d.reason == Reason::ProbeReverted).map(|d| d.target.get()).collect();
        assert!(!reverted.is_empty() && reverted.iter().all(|&at| at == 4), "{decisions:?}");
        assert!(t.probes() > 0);
    }

    #[test]
    fn a_probe_that_reads_as_well_is_kept() {
        let mut t = tuner(4);
        drive(&mut t, 40, |r| if r >= 4 { (120.0, 5.0) } else { (60.0, 5.0) });
        assert_eq!(t.target().get(), 4);
        // The cache warms: every count reads as fast, so the probe down is kept, and explored on from.
        let decisions = drive(&mut t, 120, |_| (120.0, 5.0));
        assert!(decisions.iter().any(|d| d.reason == Reason::Probe));
        assert!(t.target().get() < 4, "{}", t.target());
    }

    #[test]
    fn noisy_windows_hold() {
        let mut t = tuner(3);
        let mut flip = false;
        let decisions = drive(&mut t, 80, |_| {
            flip = !flip;
            (if flip { 30.0 } else { 300.0 }, 5.0)
        });
        assert!(decisions.iter().any(|d| d.reason == Reason::Noisy));
    }

    #[test]
    fn a_profile_whose_throughput_is_not_reached_is_distrusted() {
        let start = NonZeroUsize::new(8).unwrap();
        let fallback = NonZeroUsize::new(2).unwrap();
        let mut t = Tuner::new(TunerConfig::default(), Prior { start, expected: Some(200e6), fallback });
        let decisions = drive(&mut t, 3, |_| (100.0, 5.0));
        assert_eq!(decisions.last().map(|d| (d.reason, d.target.get())), Some((Reason::Distrusted, 2)));
    }

    #[test]
    fn the_kind_priors_are_two_four_and_three() {
        assert_eq!(Prior::of_kind(Kind::Rotational).start.get(), 2);
        assert_eq!(Prior::of_kind(Kind::Solid).start.get(), 4);
        assert_eq!(Prior::of_kind(Kind::Unknown).start.get(), 3);
    }

    #[test]
    fn what_was_learned_is_the_count_and_its_read_bound_time() {
        let mut t = tuner(2);
        drive(&mut t, 60, exos);
        let learned = t.learned();
        assert_eq!(learned.readers, t.target());
        assert!(learned.read_bound >= Duration::from_secs(60), "{learned:?}");
        assert!(learned.tested.0 <= learned.tested.1);
    }
}
