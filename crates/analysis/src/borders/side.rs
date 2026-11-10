//! One side's border: where it starts, how deep it goes, and the rules that reject it.

use pg_model::detection::{Kind, Reason, Side};

use super::lines::SolidLines;
use super::params::Params;
use super::seam::{DetailSums, refine_seam, seam_straightness};
use super::stats::{hist_median2, run_length};
use super::view::View;

/// The buffers a side's detection uses, kept from side to side and image to image.
#[derive(Default)]
pub(super) struct Scratch {
    lines: SolidLines,
    sums: DetailSums,
    /// [`blur_side`]'s line details, summed down the lines.
    act_cum: Vec<f64>,
}

impl Scratch {
    /// The bytes its buffers hold.
    pub(super) fn capacity_bytes(&self) -> usize {
        self.lines.capacity_bytes() + self.sums.capacity_bytes() + self.act_cum.capacity() * size_of::<f64>()
    }
}

/// `x` rounded toward zero, as a count of pixels or lines.
fn to_count(x: f64) -> usize {
    pg_model::num::f64_to_usize(x.clamp(0.0, f64::from(u32::MAX)))
}

/// What starts a side's detection: its first lines solid (measured into `lines`), or blurred. `blur_thr`: see
/// `blur_threshold`.
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
pub(super) fn detect_side(s: &mut Scratch, v: &View, p: &Params, blur_thr: Option<f64>) -> Option<Side> {
    let depth_max = to_count(pg_model::num::count(v.depth) * p.max_frac);
    if depth_max < p.min_px + 8 {
        return None;
    }
    match starts(v, p, blur_thr, &mut s.lines)? {
        Kind::Solid => solid_side(v, p, depth_max, &mut s.lines, &mut s.sums),
        Kind::Blur => blur_side(v, p, depth_max, &mut s.sums, &mut s.act_cum),
    }
}

fn solid_side(v: &View, p: &Params, depth_max: usize, lines: &mut SolidLines, sums: &mut DetailSums) -> Option<Side> {
    // `starts` measured the first lines.
    let [r, g, b] = lines.medians2.first()?.map(|m| u8::try_from(m / 2).ok());
    let color = [r?, g?, b?];
    let side = |depth: usize, straight: f64, rejected| {
        Some(Side {
            pass: 0,
            edge: v.edge,
            kind: Kind::Solid,
            depth: u32::try_from(depth).ok()?,
            color: Some(color),
            straight,
            dev: None,
            drift: None,
            rejected,
        })
    };
    // Look at a few lines first and widen only while the run continues.
    let mut n = 64;
    let end = loop {
        let n_eff = n.min(depth_max);
        lines.measure_to(v, n_eff, p);
        let end = run_length(lines.solid.get(..n_eff)?);
        if end + 3 < n_eff || n_eff == depth_max {
            break end;
        }
        n *= 4;
    };
    if end < p.min_px {
        return None;
    }
    if end + 3 >= depth_max {
        return side(end, 0.0, Some(Reason::TooDeep));
    }
    // JPEG ringing next to the content makes the run stop a few lines early.
    let seam = refine_seam(v, end, 3, 12);
    let s0 = seam.saturating_sub(8);
    let s1 = (seam + 9).min(v.depth);
    let off = seam - s0;
    // Thin borders: compare detail over as many lines as the border has.
    let k = 6.min(off.saturating_sub(1));
    if k < 1 || s1 - s0 < off + 1 + k {
        return side(seam, 0.0, Some(Reason::NoSeam));
    }
    sums.measure(v, s0..s1);
    let straight = seam_straightness(v, sums, seam, k, p);
    if straight < p.straight {
        return side(seam, straight, Some(Reason::SeamNotStraight));
    }
    // Flatness, away from the seam's JPEG ringing: the lines' median colours, their median, and how far pixels and
    // line colours stray from it.
    let core = seam.saturating_sub(10).max(1);
    lines.measure_to(v, core, p);
    let meds = lines.medians2.get(..core)?;
    // Per channel: a histogram of the doubled medians (0..=510), and their range.
    let mut hists = [[0u32; 511]; 3];
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for m in meds {
        for (((hist, &value), low), high) in hists.iter_mut().zip(m).zip(&mut lo).zip(&mut hi) {
            if let Some(bin) = usize::try_from(value).ok().and_then(|v| hist.get_mut(v)) {
                *bin += 1;
            }
            *low = (*low).min(value);
            *high = (*high).max(value);
        }
    }
    // Twice the median of doubled values: 4 × the median colour.
    let center4 = hists.map(|hist| hist_median2(&hist, meds.len()));
    let drift2 = lo.iter().zip(&hi).fold(0, |drift2, (&low, &high)| drift2.max(high - low));
    let mut hist = [0u32; 1021];
    let mut count = 0;
    for line in 0..core {
        for px in v.line(line) {
            let dev4 = px.iter().zip(&center4).map(|(&x, &c)| (4 * x - c).abs()).max().unwrap_or(0);
            // At most 4 × 255 (4 × a channel against 4 × a median colour): it has its bin.
            if let Some(bin) = usize::try_from(dev4).ok().and_then(|d| hist.get_mut(d)) {
                *bin += 1;
                count += 1;
            }
        }
    }
    let dev = f64::from(hist_median2(&hist, count)) / 8.0;
    let drift = f64::from(drift2) / 2.0;
    Some(Side { dev: Some(dev), drift: Some(drift), ..side((seam + p.margin).min(depth_max), straight, None)? })
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
    let mut total = 0.0;
    for line in 0..lines {
        let act: u64 = sums.between(line, line + 1)?.map(u64::from).sum();
        total += pg_model::num::size(act);
        act_cum.push(total);
    }
    let found = (first..last).find_map(|row| {
        let straight = seam_straightness(v, sums, row, K, p);
        // The mean detail of the bar above the row, and of the content below it (`last` leaves room for it).
        let (&cum_row, &cum_end) = (act_cum.get(row)?, act_cum.get(row + CONTENT)?);
        let bar = cum_row / pg_model::num::count(row);
        let content = (cum_end - cum_row) / pg_model::num::count(CONTENT);
        (straight >= p.blur_straight && bar < 0.35 * content).then_some((row, straight))
    });
    let (row, straight) = found?;
    let seam = refine_seam(v, row, 1, 3);
    Some(Side {
        pass: 0,
        edge: v.edge,
        kind: Kind::Blur,
        depth: u32::try_from((seam + p.margin).min(depth_max)).ok()?,
        color: None,
        straight,
        dev: None,
        drift: None,
        rejected: None,
    })
}

/// The rules that need the opposite side (`other`: its accepted detection); rejects `side` in place.
pub(super) fn judge(side: &mut Side, other: Option<&Side>, p: &Params) {
    match side.kind {
        Kind::Blur => {
            let paired = other.is_some_and(|o| {
                let (a, b) = (f64::from(side.depth), f64::from(o.depth));
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
