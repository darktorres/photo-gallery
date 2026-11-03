//! Borders around images (FINDINGS "Border cleaning"): solid ones (black, white, colored, JPEG-noisy) and blurred
//! padding bars (a blurred, enlarged copy of the image filling the letterbox). A port of the Python prototype that
//! measured them (`tools/borders.py`, removed once this matched it file for file; in git history), run on the stored
//! pixels at full size (no EXIF orientation: a crop of the stored image keeps the tag
//! valid), by a pass of its own after the import (`pg_ingest::borders`).
//!
//! Per side (top, bottom, left, right; repeated so nested borders peel off):
//! - A line is *solid* if nearly all its pixels lie within `tol` of the line's median colour, and that colour changes
//!   only gradually from line to line (absorbs JPEG noise and ringing).
//! - A line is *blurred* if its detail (mean gradient) is far below the image's central detail.
//! - The border is the run of such lines from the edge; the seam is the first strong line-to-line change near its end.
//! - Accepted only if the seam is a straight edge across most of the line (a dark sky or a bokeh background ends
//!   irregularly), and, for a blurred bar, if the content past the seam is clearly sharper.
//! - Then the rules that need the opposite side (`judge`): solid padding is digitally flat, and blurred bars come
//!   in equal pairs.
//!
//! The arithmetic is exact where Python's was float32: grays are kept as sums of the three channels (×3), medians of
//! 8-bit values doubled (×2), deviations from a median of medians ×4. Line statistics read every other pixel.

use crate::detection::{Detection, Edge, Kind, MAX_SIDES, Reason, Side, Sides};
use crate::pixels::Rgb;

/// Bump when anything here changes what is detected; detections of an older version are redone.
pub const BORDERS_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Max per-channel distance from the line colour that still counts as border.
    pub tol: i32,
    /// Max fraction of a solid line's pixels beyond `tol`.
    pub solid_off: f64,
    /// A blurred line has less detail than this fraction of the image's central detail…
    pub blur_rel: f64,
    /// …or than this mean gradient (gray levels per pixel).
    pub blur_abs: f64,
    /// Below this central detail, an image is smooth overall and blurred bars aren't looked for.
    pub min_content_act: f64,
    /// Thinner borders are ignored.
    pub min_px: usize,
    /// A border covers at most this fraction of a side.
    pub max_frac: f64,
    /// The content keeps at least this fraction of each dimension.
    pub min_keep: f64,
    /// Min fraction of the seam that must be a sharp straight edge…
    pub straight: f64,
    /// …for blurred bars, which have no colour edge to rely on.
    pub blur_straight: f64,
    /// Blurred bars on opposite sides may differ in depth by this fraction.
    pub blur_sym: f64,
    /// Per-pixel change that marks a column as crossing the seam.
    pub seam_step: i32,
    /// Real padding is digitally flat away from the seam; a wall or sky has shading. Max median pixel deviation of
    /// a one-sided solid border…
    pub flat_dev: f64,
    /// …of a solid border when a side is detected opposite it (of any kind; `judge` checks `other.is_some()`)…
    pub flat_dev_paired: f64,
    /// …and max drift of the line colour across the border's depth (JPEG noise moves pixels, not line medians).
    pub max_drift: f64,
    /// Extra pixels removed past each seam (JPEG chroma blending at the edge).
    pub margin: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            tol: 24,
            solid_off: 0.02,
            blur_rel: 0.15,
            blur_abs: 1.0,
            min_content_act: 3.0,
            min_px: 3,
            max_frac: 0.45,
            min_keep: 0.3,
            straight: 0.5,
            blur_straight: 0.7,
            blur_sym: 0.1,
            seam_step: 12,
            flat_dev: 4.0,
            flat_dev_paired: 8.0,
            max_drift: 8.0,
            margin: 1,
        }
    }
}

/// Find the borders of a stored image, once. A pass over many images keeps a [`Detector`] instead.
pub fn detect(img: &Rgb, p: &Params) -> Detection {
    Detector::default().detect(img, p)
}

/// The border detector's buffers, kept from image to image: each grows to what the largest image so far needed.
#[derive(Default)]
pub struct Detector {
    lines: SolidLines,
    sums: DetailSums,
    /// [`blur_side`]'s line details, summed down the lines.
    act_cum: Vec<f64>,
    /// [`blur_threshold`]'s line details.
    details: Vec<f64>,
}

impl Detector {
    /// Find the borders of a stored image.
    pub fn detect(&mut self, img: &Rgb, p: &Params) -> Detection {
        detect_with(self, img, p)
    }

    /// The bytes its buffers hold.
    pub fn capacity_bytes(&self) -> usize {
        self.lines.medians2.capacity() * size_of::<[i32; 3]>()
            + self.lines.solid.capacity()
            + self.sums.cum.capacity() * size_of::<u32>()
            + (self.act_cum.capacity() + self.details.capacity()) * size_of::<f64>()
    }

    /// Frees its buffers if they hold more than `bytes` (after an image far larger than the rest).
    pub fn trim(&mut self, bytes: usize) {
        if self.capacity_bytes() > bytes {
            *self = Detector::default();
        }
    }
}

/// The rounds of peeling ([`MAX_SIDES`] counts them).
const PASSES: u8 = 3;
const _: () = assert!(MAX_SIDES == PASSES as usize * 4);

fn detect_with(d: &mut Detector, img: &Rgb, p: &Params) -> Detection {
    let (w, h) = (img.width, img.height);
    let blur_thr = blur_threshold(img, p, &mut d.details);
    let mut content = [0, 0, w, h];
    let mut sides = Sides::new();
    // Nested borders, e.g. a thin line inside a blurred bar. Each round finds at most one side of each edge, so
    // `sides` never holds more than `MAX_SIDES`.
    for pass in 0..PASSES {
        let mut changed = false;
        // Top/bottom first, then left/right on what is left, so corners never mix two borders.
        for pair in [[Edge::Top, Edge::Bottom], [Edge::Left, Edge::Right]] {
            let found = pair.map(|edge| {
                detect_side(d, &View::new(img, content, edge), p, blur_thr).map(|mut s| {
                    s.pass = pass;
                    s
                })
            });
            // The opposite side's accepted detection, captured before either is judged, so the outcome doesn't
            // depend on which is judged first.
            let detected = found.clone().map(|s| s.filter(Side::accepted));
            let mut found = found;
            for i in 0..2 {
                if let Some(side) = found[i].as_mut().filter(|s| s.accepted()) {
                    judge(side, detected[1 - i].as_ref(), p);
                }
            }
            for side in found.into_iter().flatten() {
                if !side.accepted() {
                    if !sides.iter().any(|s| !s.accepted() && s.edge == side.edge && s.depth == side.depth) {
                        sides.push(side);
                    }
                    continue;
                }
                let depth = side.depth as usize;
                match side.edge {
                    Edge::Top => content[1] += depth,
                    Edge::Bottom => content[3] -= depth,
                    Edge::Left => content[0] += depth,
                    Edge::Right => content[2] -= depth,
                }
                sides.push(side);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let [l, t, r, b] = content;
    let any = sides.iter().any(Side::accepted);
    if any && (((r - l) as f64) < p.min_keep * w as f64 || ((b - t) as f64) < p.min_keep * h as f64) {
        for s in sides.iter_mut().filter(|s| s.accepted()) {
            s.rejected = Some(Reason::TooLittleLeft);
        }
        content = [0, 0, w, h];
    }
    Detection { width: w as u32, height: h as u32, content: content.map(|v| v as u32), sides }
}

/// The detail below which a line is blurred: a fraction of the image's central detail. `None` for images that are
/// smooth overall, where blurred bars aren't looked for.
fn blur_threshold(img: &Rgb, p: &Params, details: &mut Vec<f64>) -> Option<f64> {
    let (w, h) = (img.width, img.height);
    // Every 4th line of the middle third, every other pixel of the middle three quarters.
    let (x0, x1) = (w / 8, 7 * w / 8);
    let xs = (x0..x1).step_by(2);
    let n = xs.len();
    details.clear();
    if n > 1 {
        details.extend((h / 3..2 * h / 3).step_by(4).map(|y| {
            let sum = |x: usize| {
                let i = (y * w + x) * 3;
                img.pixels[i] as i32 + img.pixels[i + 1] as i32 + img.pixels[i + 2] as i32
            };
            let mut prev = sum(x0);
            let total: i64 = xs
                .clone()
                .skip(1)
                .map(|x| {
                    let g = sum(x);
                    let step = (g - prev).abs() as i64;
                    prev = g;
                    step
                })
                .sum();
            total as f64 / (3.0 * (n - 1) as f64)
        }));
    }
    let content_act = median_f64(details).unwrap_or(0.0);
    (content_act >= p.min_content_act).then(|| p.blur_abs.max(p.blur_rel * content_act))
}

/// One side of the image's current content box, seen as if it were the top edge: `line` counts from the edge
/// inwards, `pos` along the edge.
struct View<'a> {
    img: &'a Rgb,
    edge: Edge,
    box_: [usize; 4],
    /// Lines from the edge to the opposite one.
    depth: usize,
    /// The positions of a line read (every other one).
    positions: usize,
}

impl<'a> View<'a> {
    fn new(img: &'a Rgb, box_: [usize; 4], edge: Edge) -> View<'a> {
        let [l, t, r, b] = box_;
        let (depth, len) = match edge {
            Edge::Top | Edge::Bottom => (b - t, r - l),
            Edge::Left | Edge::Right => (r - l, b - t),
        };
        View { img, edge, box_, depth, positions: len.div_ceil(2) }
    }

    /// The pixel at a line and a read position (`k`-th of every other one).
    #[inline]
    fn at(&self, line: usize, k: usize) -> [i32; 3] {
        let [l, t, r, b] = self.box_;
        let pos = 2 * k;
        let (x, y) = match self.edge {
            Edge::Top => (l + pos, t + line),
            Edge::Bottom => (l + pos, b - 1 - line),
            Edge::Left => (l + line, t + pos),
            Edge::Right => (r - 1 - line, t + pos),
        };
        let i = (y * self.img.width + x) * 3;
        let px = &self.img.pixels[i..i + 3];
        [px[0] as i32, px[1] as i32, px[2] as i32]
    }

    /// A read pixel as the sum of its channels (3 × gray).
    #[inline]
    fn gray3(&self, line: usize, k: usize) -> i32 {
        self.at(line, k).iter().sum()
    }

    /// A line's steps between neighbouring read pixels, 3 × |gray step|, in order (`positions - 1` of them).
    fn steps3(&self, line: usize) -> impl Iterator<Item = u32> + '_ {
        let mut prev = if self.positions > 0 { self.gray3(line, 0) } else { 0 };
        (1..self.positions).map(move |k| {
            let g = self.gray3(line, k);
            let step = (g - prev).unsigned_abs();
            prev = g;
            step
        })
    }

    /// A line's detail: its steps, summed.
    fn detail3(&self, line: usize) -> i64 {
        self.steps3(line).map(i64::from).sum()
    }

    /// Mean gray step of a line (`borders.py`'s `detail`).
    fn detail(&self, line: usize) -> f64 {
        self.detail3(line) as f64 / (3.0 * (self.positions.max(2) - 1) as f64)
    }
}

/// Per line: twice its median colour, and whether it is solid (see the module comment). Each line depends only on
/// itself and the line before, so the lines are measured as far as they are asked for.
#[derive(Default)]
struct SolidLines {
    medians2: Vec<[i32; 3]>,
    solid: Vec<bool>,
}

impl SolidLines {
    /// Start over, for another side.
    fn clear(&mut self) {
        self.medians2.clear();
        self.solid.clear();
    }

    fn measure_to(&mut self, v: &View, n: usize, p: &Params) {
        while self.solid.len() < n {
            let line = self.solid.len();
            let m2 = median2(v, line);
            let off = (0..v.positions)
                .filter(|&k| {
                    let px = v.at(line, k);
                    (0..3).any(|c| (2 * px[c] - m2[c]).abs() > 2 * p.tol)
                })
                .count();
            let off_ok = (off as f64 / v.positions as f64) < p.solid_off;
            // The colour may change from line to line by tol / 3 at most.
            let jump_ok =
                self.medians2.last().is_none_or(|prev| (0..3).all(|c| 3 * (m2[c] - prev[c]).abs() <= 2 * p.tol));
            self.medians2.push(m2);
            self.solid.push(off_ok && jump_ok);
        }
    }
}

/// Twice the per-channel median of a line's read pixels (numpy's: the mean of the middle two of an even count), from
/// histograms.
fn median2(v: &View, line: usize) -> [i32; 3] {
    let mut hists = [[0u32; 256]; 3];
    for k in 0..v.positions {
        let px = v.at(line, k);
        for (hist, value) in hists.iter_mut().zip(px) {
            hist[value as usize] += 1;
        }
    }
    hists.map(|hist| hist_median2(&hist, v.positions))
}

/// Twice the median of `n` values counted in `hist` (value = index).
fn hist_median2(hist: &[u32], n: usize) -> i32 {
    let nth = |k: usize| {
        let mut seen = 0usize;
        hist.iter()
            .position(|&count| {
                seen += count as usize;
                seen > k
            })
            .unwrap_or(0) as i32
    };
    if n == 0 {
        0
    } else if n % 2 == 1 {
        2 * nth(n / 2)
    } else {
        nth(n / 2 - 1) + nth(n / 2)
    }
}

/// The median of `values` (numpy's: the mean of the middle two of an even count), which it reorders. The middle
/// values are selected rather than sorted (`total_cmp`'s order, as a sort by it gives: values it calls equal are
/// the same bits), so it takes linear time and allocates nothing.
fn median_f64(values: &mut [f64]) -> Option<f64> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let (below, &mut upper, _) = values.select_nth_unstable_by(n / 2, f64::total_cmp);
    Some(if n % 2 == 1 {
        upper
    } else {
        // The lower middle value is the largest of those before the upper one.
        let lower = below.iter().copied().max_by(f64::total_cmp).expect("n is even and not 0");
        (lower + upper) / 2.0
    })
}

/// Length of the leading run of `true`, bridging gaps of up to `max_gap` if the run then resumes for `resume`.
fn run_length(mask: &[bool]) -> usize {
    const MAX_GAP: usize = 2;
    const RESUME: usize = 3;
    let n = mask.len();
    let mut i = 0;
    while i < n {
        if mask[i] {
            i += 1;
            continue;
        }
        let next = (i..(i + MAX_GAP + 1).min(n)).find(|&j| mask[j]);
        match next {
            Some(j) if j + RESUME <= n && mask[j..j + RESUME].iter().all(|&m| m) => i = j,
            _ => break,
        }
    }
    i
}

/// [`refine_seam`]'s widest span: `before + after + 1` lines.
const SEAM_SPAN: usize = 16;

/// The first strong line-to-line change in `[guess - before, guess + after]`.
fn refine_seam(v: &View, guess: usize, before: usize, after: usize) -> usize {
    assert!(before + after < SEAM_SPAN, "a seam is looked for in at most {SEAM_SPAN} lines");
    let lo = guess.saturating_sub(before).max(1);
    let hi = (guess + after).min(v.depth - 1);
    if hi < lo {
        return guess;
    }
    // step[k]: the mean (over read positions) of the largest channel change into line lo + k.
    let mut steps = [0.0f64; SEAM_SPAN];
    let steps = &mut steps[..=hi - lo];
    for (step, line) in steps.iter_mut().zip(lo..=hi) {
        let total: i64 = (0..v.positions)
            .map(|k| {
                let (a, b) = (v.at(line - 1, k), v.at(line, k));
                (0..3).map(|c| (b[c] - a[c]).abs()).max().unwrap_or(0) as i64
            })
            .sum();
        *step = total as f64 / v.positions as f64;
    }
    let max = steps.iter().copied().fold(0.0, f64::max);
    let strong = 3.0f64.max(0.5 * max);
    steps.iter().position(|&s| s >= strong).map_or(guess, |k| lo + k)
}

/// Per-line horizontal detail per column, 3 × |gray step|, summed down the lines: row `i` of `cum` (`cols` wide) is
/// the sum over lines `first..first + i` at each column step.
#[derive(Default)]
struct DetailSums {
    first: usize,
    cols: usize,
    cum: Vec<u32>,
}

impl DetailSums {
    /// Measure `lines` of `v`, replacing what was measured before.
    fn measure(&mut self, v: &View, lines: std::ops::Range<usize>) {
        let cols = v.positions.saturating_sub(1);
        self.first = lines.start;
        self.cols = cols;
        self.cum.clear();
        self.cum.reserve((lines.len() + 1) * cols);
        self.cum.resize(cols, 0);
        for (i, line) in lines.enumerate() {
            let prev = i * cols;
            for (j, step) in v.steps3(line).enumerate() {
                let next = self.cum[prev + j] + step;
                self.cum.push(next);
            }
        }
    }

    /// The sum over lines `a..b` at column step `j`.
    fn sum(&self, a: usize, b: usize, j: usize) -> u32 {
        self.cum[(b - self.first) * self.cols + j] - self.cum[(a - self.first) * self.cols + j]
    }
}

/// For a candidate first content line `row`: the fraction of columns where it is a sharp edge. A column counts if
/// its colour jumps across `row`, or its detail does (sharp content below a smooth border), comparing the `k` lines
/// above with the `k` below. Real borders end in a straight edge; skies and bokeh don't.
fn seam_straightness(v: &View, sums: &DetailSums, row: usize, k: usize, p: &Params) -> f64 {
    let cols = v.positions.saturating_sub(1);
    if cols == 0 {
        return 0.0;
    }
    let sharp = (0..cols)
        .filter(|&j| {
            let (below, above) = (v.at(row + 1, j), v.at(row - 2, j));
            let color = (0..3).any(|c| (below[c] - above[c]).abs() > p.seam_step);
            // Detail below > 2 × detail above + 2, in sums of 3 × gray over k lines.
            let a = sums.sum(row - 1 - k, row - 1, j) as i64;
            let b = sums.sum(row + 1, row + 1 + k, j) as i64;
            color || b > 2 * a + 6 * k as i64
        })
        .count();
    sharp as f64 / cols as f64
}

/// What starts a side's detection: its first lines solid (measured into `lines`), or blurred. `blur_thr`: see
/// [`blur_threshold`].
fn starts(v: &View, p: &Params, blur_thr: Option<f64>, lines: &mut SolidLines) -> Option<Kind> {
    if v.depth < 4 || v.positions < 2 {
        return None;
    }
    lines.clear();
    lines.measure_to(v, 4, p);
    if lines.solid.iter().all(|&s| s) {
        return Some(Kind::Solid);
    }
    blurred(v, blur_thr).then_some(Kind::Blur)
}

/// Whether the first four lines have less detail than `blur_thr`.
fn blurred(v: &View, blur_thr: Option<f64>) -> bool {
    blur_thr.is_some_and(|thr| (0..4).map(|line| v.detail(line)).sum::<f64>() / 4.0 < thr)
}

/// A border at the edge of `v`, if any (accepted, or a rejected candidate).
fn detect_side(d: &mut Detector, v: &View, p: &Params, blur_thr: Option<f64>) -> Option<Side> {
    let depth_max = (v.depth as f64 * p.max_frac) as usize;
    if depth_max < p.min_px + 8 {
        return None;
    }
    match starts(v, p, blur_thr, &mut d.lines)? {
        Kind::Solid => solid_side(v, p, depth_max, &mut d.lines, &mut d.sums),
        Kind::Blur => blur_side(v, p, depth_max, &mut d.sums, &mut d.act_cum),
    }
}

fn solid_side(v: &View, p: &Params, depth_max: usize, lines: &mut SolidLines, sums: &mut DetailSums) -> Option<Side> {
    let m0 = lines.medians2[0];
    let color = [0, 1, 2].map(|c| (m0[c] / 2) as u8);
    let side = |depth: usize, straight: f64, rejected| Side {
        pass: 0,
        edge: v.edge,
        kind: Kind::Solid,
        depth: depth as u32,
        color: Some(color),
        straight,
        dev: None,
        drift: None,
        rejected,
    };
    // Look at a few lines first and widen only while the run continues.
    let mut n = 64;
    let end = loop {
        let n_eff = n.min(depth_max);
        lines.measure_to(v, n_eff, p);
        let end = run_length(&lines.solid[..n_eff]);
        if end + 3 < n_eff || n_eff == depth_max {
            break end;
        }
        n *= 4;
    };
    if end < p.min_px {
        return None;
    }
    if end + 3 >= depth_max {
        return Some(side(end, 0.0, Some(Reason::TooDeep)));
    }
    // JPEG ringing next to the content makes the run stop a few lines early.
    let seam = refine_seam(v, end, 3, 12);
    let s0 = seam.saturating_sub(8);
    let s1 = (seam + 9).min(v.depth);
    let off = seam - s0;
    // Thin borders: compare detail over as many lines as the border has.
    let k = 6.min(off.saturating_sub(1));
    if k < 1 || s1 - s0 < off + 1 + k {
        return Some(side(seam, 0.0, Some(Reason::NoSeam)));
    }
    sums.measure(v, s0..s1);
    let straight = seam_straightness(v, sums, seam, k, p);
    if straight < p.straight {
        return Some(side(seam, straight, Some(Reason::SeamNotStraight)));
    }
    // Flatness, away from the seam's JPEG ringing: the lines' median colours, their median, and how far pixels and
    // line colours stray from it.
    let core = seam.saturating_sub(10).max(1);
    lines.measure_to(v, core, p);
    let meds = &lines.medians2[..core];
    let mut center4 = [0i32; 3];
    let mut drift2 = 0;
    for c in 0..3 {
        let mut hist = [0u32; 511];
        for m in meds {
            hist[m[c] as usize] += 1;
        }
        // Twice the median of doubled values: 4 × the median colour.
        center4[c] = hist_median2(&hist, meds.len());
        let (lo, hi) = meds.iter().fold((i32::MAX, i32::MIN), |(lo, hi), m| (lo.min(m[c]), hi.max(m[c])));
        drift2 = drift2.max(hi - lo);
    }
    let mut hist = [0u32; 1021];
    let mut count = 0;
    for line in 0..core {
        for kpos in 0..v.positions {
            let px = v.at(line, kpos);
            let dev4 = (0..3).map(|c| (4 * px[c] - center4[c]).abs()).max().unwrap_or(0);
            hist[dev4 as usize] += 1;
            count += 1;
        }
    }
    let dev = hist_median2(&hist, count) as f64 / 8.0;
    let drift = drift2 as f64 / 2.0;
    Some(Side { dev: Some(dev), drift: Some(drift), ..side((seam + p.margin).min(depth_max), straight, None) })
}

/// A blurred bar has no fixed colour and may hold a blurred copy of the content, so instead of following a run of
/// similar lines, find the first straight sharp seam below a smooth region.
fn blur_side(v: &View, p: &Params, depth_max: usize, sums: &mut DetailSums, act_cum: &mut Vec<f64>) -> Option<Side> {
    const K: usize = 6;
    const CONTENT: usize = 64;
    let lines = (depth_max + 72).min(v.depth);
    let first = p.min_px.max(7);
    let last = depth_max.min(lines.saturating_sub(72));
    if first >= last {
        return None;
    }
    sums.measure(v, 0..lines);
    let sums = &*sums;
    // Each line's detail (as the sum over its columns: only ratios of them are compared), summed down the lines.
    act_cum.clear();
    act_cum.reserve(lines + 1);
    act_cum.push(0.0);
    for line in 0..lines {
        let act: u64 = (0..v.positions - 1).map(|j| sums.sum(line, line + 1, j) as u64).sum();
        act_cum.push(act_cum[line] + act as f64);
    }
    let found = (first..last).find_map(|row| {
        let straight = seam_straightness(v, sums, row, K, p);
        // The mean detail of the bar above the row, and of the content below it.
        let bar = act_cum[row] / row as f64;
        let content = (act_cum[row + CONTENT] - act_cum[row]) / CONTENT as f64;
        (straight >= p.blur_straight && bar < 0.35 * content).then_some((row, straight))
    });
    let (row, straight) = found?;
    let seam = refine_seam(v, row, 1, 3);
    Some(Side {
        pass: 0,
        edge: v.edge,
        kind: Kind::Blur,
        depth: (seam + p.margin).min(depth_max) as u32,
        color: None,
        straight,
        dev: None,
        drift: None,
        rejected: None,
    })
}

/// The rules that need the opposite side (`other`: its accepted detection); rejects `side` in place.
fn judge(side: &mut Side, other: Option<&Side>, p: &Params) {
    match side.kind {
        Kind::Blur => {
            let paired = other.is_some_and(|o| {
                let (a, b) = (side.depth as f64, o.depth as f64);
                o.kind == Kind::Blur && (a - b).abs() <= 4f64.max(p.blur_sym * a.max(b))
            });
            if !paired {
                side.rejected = Some(Reason::UnpairedBlur);
            }
        }
        Kind::Solid => {
            let max_dev = if other.is_some() { p.flat_dev_paired } else { p.flat_dev };
            if side.drift.unwrap_or(0.0) > p.max_drift || side.dev.unwrap_or(0.0) > max_dev {
                side.rejected = Some(Reason::NotFlat);
            }
        }
    }
    debug_assert!(other.is_none_or(|o| o.edge == side.edge.opposite()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detection::Class;

    /// Detailed content: deterministic noise in 6 px blocks, a little per pixel, over a gradient (a hash, so the
    /// tests need no crate).
    fn content(w: usize, h: usize, seed: u32) -> Rgb {
        let hash = |a: usize, b: usize, c: usize| {
            let mut v =
                seed ^ (a as u32).wrapping_mul(0x9E37_79B9) ^ (b as u32).wrapping_mul(0x85EB_CA6B) ^ (c as u32) << 28;
            v ^= v >> 15;
            v = v.wrapping_mul(0x2C1B_3C6D);
            v ^= v >> 12;
            v = v.wrapping_mul(0x297A_2D39);
            (v ^ (v >> 15)) as usize
        };
        let mut pixels = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let block = hash(x / 6, y / 6, c) % 110;
                    let fine = hash(x + 7919, y, c) % 16;
                    pixels.push((40 + block + fine + (x + y + 30 * c) % 60) as u8);
                }
            }
        }
        Rgb { width: w, height: h, pixels }
    }

    fn set(img: &mut Rgb, x: usize, y: usize, px: [u8; 3]) {
        let i = (y * img.width + x) * 3;
        img.pixels[i..i + 3].copy_from_slice(&px);
    }

    fn fill(img: &mut Rgb, x0: usize, y0: usize, x1: usize, y1: usize, px: impl Fn(usize, usize) -> [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                set(img, x, y, px(x, y));
            }
        }
    }

    fn accepted(d: &Detection) -> Vec<(Edge, Kind, u32)> {
        d.sides.iter().filter(|s| s.accepted()).map(|s| (s.edge, s.kind, s.depth)).collect()
    }

    fn rejected(d: &Detection) -> Vec<(Edge, Reason)> {
        d.sides.iter().filter_map(|s| s.rejected.map(|r| (s.edge, r))).collect()
    }

    #[test]
    fn black_letterbox() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 1);
        fill(&mut img, 0, 0, w, 40, |_, _| [0, 0, 0]);
        fill(&mut img, 0, h - 40, w, h, |_, _| [2, 1, 0]);
        let d = detect(&img, &Params::default());
        // The seam is at 40, plus one pixel of margin.
        assert_eq!(accepted(&d), vec![(Edge::Top, Kind::Solid, 41), (Edge::Bottom, Kind::Solid, 41)]);
        assert_eq!(d.content, [0, 41, 400, 259]);
        assert_eq!(d.sides[0].class(), Class::Black);
    }

    #[test]
    fn white_side_padding() {
        let (w, h) = (500, 300);
        let mut img = content(w, h, 2);
        fill(&mut img, 0, 0, 90, h, |_, _| [255, 255, 255]);
        fill(&mut img, w - 90, 0, w, h, |_, _| [253, 254, 255]);
        let d = detect(&img, &Params::default());
        assert_eq!(accepted(&d), vec![(Edge::Left, Kind::Solid, 91), (Edge::Right, Kind::Solid, 91)]);
        assert_eq!(d.sides[0].class(), Class::White);
    }

    #[test]
    fn thin_dark_frame() {
        let (w, h) = (320, 240);
        let mut img = content(w, h, 3);
        let frame = |x: usize, y: usize| x < 5 || y < 5 || x >= w - 5 || y >= h - 5;
        fill(&mut img, 0, 0, w, h, |x, y| if frame(x, y) { [20, 18, 22] } else { [0; 3] });
        let inner = content(w, h, 3);
        for y in 5..h - 5 {
            for x in 5..w - 5 {
                let i = (y * w + x) * 3;
                let px = [inner.pixels[i], inner.pixels[i + 1], inner.pixels[i + 2]];
                set(&mut img, x, y, px);
            }
        }
        let d = detect(&img, &Params::default());
        let found = accepted(&d);
        assert_eq!(found.len(), 4, "{:?}", d.sides);
        assert!(found.iter().all(|&(_, k, depth)| k == Kind::Solid && (5..=7).contains(&depth)), "{found:?}");
    }

    /// A smooth bar: colour varying slowly along the line, so it is no solid line, and almost no detail.
    fn bar(x: usize, y: usize) -> [u8; 3] {
        let t = (x as f64 / 120.0).sin() * 60.0 + (y as f64 / 50.0).cos() * 10.0;
        [(110.0 + t) as u8, (100.0 + t * 0.8) as u8, (90.0 + t * 0.5) as u8]
    }

    #[test]
    fn blurred_bars_in_pairs() {
        let (w, h) = (300, 520);
        let mut img = content(w, h, 4);
        fill(&mut img, 0, 0, w, 110, bar);
        fill(&mut img, 0, h - 110, w, h, bar);
        let d = detect(&img, &Params::default());
        let found = accepted(&d);
        assert_eq!(found.len(), 2, "{:?}", d.sides);
        // The seam is found where the detail below the row jumps, which a few lines of content already make; the
        // exact depth is pinned by the parity check against borders.py (FINDINGS), not here.
        assert!(found.iter().all(|&(_, k, depth)| k == Kind::Blur && (100..=113).contains(&depth)), "{found:?}");
        assert_eq!(found[0].2, found[1].2);
    }

    #[test]
    fn one_blurred_bar_is_a_sky() {
        let (w, h) = (300, 520);
        let mut img = content(w, h, 5);
        fill(&mut img, 0, 0, w, 110, bar);
        let d = detect(&img, &Params::default());
        assert!(!d.found(), "{:?}", d.sides);
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::UnpairedBlur)]);
        assert_eq!(d.content, [0, 0, w as u32, h as u32]);
    }

    #[test]
    fn shaded_wall_is_not_flat() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 6);
        // Each line uniform, the colour drifting down: a wall lit from above.
        fill(&mut img, 0, 0, w, 70, |_, y| [(100 + y) as u8, (110 + y) as u8, (120 + y) as u8]);
        let d = detect(&img, &Params::default());
        assert!(!d.found(), "{:?}", d.sides);
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::NotFlat)]);
    }

    #[test]
    fn too_deep() {
        let (w, h) = (400, 300);
        let mut img = content(w, h, 7);
        fill(&mut img, 0, 0, w, 150, |_, _| [0, 0, 0]);
        let d = detect(&img, &Params::default());
        assert!(!d.found());
        assert_eq!(rejected(&d), vec![(Edge::Top, Reason::TooDeep)]);
    }

    #[test]
    fn no_border() {
        let img = content(400, 300, 8);
        let d = detect(&img, &Params::default());
        assert!(d.sides.is_empty(), "{:?}", d.sides);
    }

    /// The median as a sort by `total_cmp` gives it, for every length up to 9 and random values, with ties, both
    /// zeros, infinities and NaNs among them.
    #[test]
    fn median_is_the_sorted_one() {
        fn sorted_median(values: &[f64]) -> Option<f64> {
            let mut v = values.to_vec();
            v.sort_by(f64::total_cmp);
            let n = v.len();
            (n > 0).then(|| if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
        }
        let specials = [0.0, -0.0, 1.0, 1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN, 0.5, 1e-300];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let n = (next() % 10) as usize;
            let values: Vec<f64> = (0..n)
                .map(|_| match next() % 3 {
                    0 => specials[(next() % specials.len() as u64) as usize],
                    1 => (next() % 7) as f64,
                    _ => f64::from_bits(next()),
                })
                .collect();
            let want = sorted_median(&values);
            let got = median_f64(&mut values.clone());
            assert_eq!(got.map(f64::to_bits), want.map(f64::to_bits), "{values:?}");
        }
    }

    #[test]
    fn nested_borders_peel_off() {
        // Black letterbox bars, and inside them a white frame around the content. Above and below, the run of
        // border lines bridges the colour change (as borders.py's run_length does): one side each.
        let (w, h) = (400, 400);
        let mut img = content(w, h, 9);
        fill(&mut img, 0, 0, w, h, |x, y| {
            if y < 30 || y >= h - 30 {
                [0, 0, 0]
            } else if y < 36 || y >= h - 36 || x < 6 || x >= w - 6 {
                [250, 250, 250]
            } else {
                [0; 3]
            }
        });
        let inner = content(w, h, 9);
        for y in 36..h - 36 {
            for x in 6..w - 6 {
                let i = (y * w + x) * 3;
                set(&mut img, x, y, [inner.pixels[i], inner.pixels[i + 1], inner.pixels[i + 2]]);
            }
        }
        let d = detect(&img, &Params::default());
        let [l, t, r, b] = d.content;
        assert!((6..=8).contains(&l) && (w as u32 - 8..=w as u32 - 6).contains(&r), "{:?} {:?}", d.content, d.sides);
        assert!(
            (36..=39).contains(&t) && (h as u32 - 39..=h as u32 - 36).contains(&b),
            "{:?} {:?}",
            d.content,
            d.sides
        );
    }

    #[test]
    fn too_little_content_left() {
        // Letterbox bars of 36% each: each within the 45% a side may cut, together leaving 28% of the height.
        let (w, h) = (400, 400);
        let mut img = content(w, h, 10);
        fill(&mut img, 0, 0, w, 144, |_, _| [0, 0, 0]);
        fill(&mut img, 0, h - 144, w, h, |_, _| [0, 0, 0]);
        let d = detect(&img, &Params::default());
        assert!(!d.found(), "{:?}", d.sides);
        assert!(d.sides.iter().any(|s| s.rejected == Some(Reason::TooLittleLeft)));
        assert_eq!(d.content, [0, 0, w as u32, h as u32]);
    }

    #[test]
    fn a_kept_detector_finds_what_a_new_one_does() {
        // Images of different sizes and borders, in an order where each leaves larger buffers than the next needs.
        let mut images = Vec::new();
        let mut letterbox = content(400, 300, 11);
        fill(&mut letterbox, 0, 0, 400, 40, |_, _| [0, 0, 0]);
        fill(&mut letterbox, 0, 260, 400, 300, |_, _| [0, 0, 0]);
        let mut bars = content(300, 520, 12);
        fill(&mut bars, 0, 0, 300, 110, bar);
        fill(&mut bars, 0, 410, 300, 520, bar);
        let mut padded = content(500, 300, 13);
        fill(&mut padded, 0, 0, 90, 300, |_, _| [255, 255, 255]);
        let mut small = content(120, 90, 14);
        fill(&mut small, 0, 0, 120, 12, |_, _| [30, 60, 90]);
        images.extend([bars, letterbox, content(400, 300, 15), padded, small]);
        let mut kept = Detector::default();
        for _ in 0..2 {
            for img in &images {
                assert_eq!(kept.detect(img, &Params::default()), detect(img, &Params::default()));
            }
        }
        assert!(kept.capacity_bytes() > 0);
        kept.trim(0);
        assert_eq!(kept.capacity_bytes(), 0);
    }

    #[test]
    fn run_length_bridges_short_gaps() {
        let t = true;
        let f = false;
        assert_eq!(run_length(&[t, t, f, t, t, t, f]), 6);
        assert_eq!(run_length(&[t, t, f, f, f, t, t, t]), 2);
        assert_eq!(run_length(&[t, t, f, t, t]), 2);
        assert_eq!(run_length(&[f, t, t, t]), 4);
    }

    #[test]
    fn medians_match_numpy() {
        let mut hist = [0u32; 256];
        for v in [3, 1, 2, 10] {
            hist[v] += 1;
        }
        // numpy.median([1, 2, 3, 10]) = 2.5
        assert_eq!(hist_median2(&hist, 4), 5);
        hist[7] += 1;
        assert_eq!(hist_median2(&hist, 5), 6);
    }
}
