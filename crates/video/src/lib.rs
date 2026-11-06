//! Videos, through FFmpeg (our own pinned build, which Qt Multimedia plays them with too: crates/README.md
//! "Build"): what the import stores about one, and the frame its ThumbHash and thumbnails are made from.
//!
//! "Upright" is as players show it: the stream's display matrix (a phone's rotation tag) applied, then the
//! catalog rotation. Near-duplicate detection doesn't cover videos yet: no pixel hash or PDQ.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

mod ffi;

use pg_model::turns::Turns;
use std::ffi::c_int;
use std::path::Path;
use std::sync::Once;

use anyhow::{Context, Result, bail};

use ffi::{
    AV_LOG_FATAL, AVColorRange_AVCOL_RANGE_JPEG, AVColorRange_AVCOL_RANGE_MPEG, AVColorRange_AVCOL_RANGE_UNSPECIFIED,
    AVColorSpace_AVCOL_SPC_BT470BG, AVColorSpace_AVCOL_SPC_BT709, AVColorSpace_AVCOL_SPC_UNSPECIFIED,
    AVPixelFormat_AV_PIX_FMT_RGB24, Decoder, Frame, Input, NOPTS_VALUE, Packet, Scaler, Stream,
};

use pg_codec::decode::orient_into;
use pg_codec::pixels::Rgb;
use pg_model::analysis::Taken;
use pg_model::date::parse_iso_date;
use pg_model::orientation::Orientation;

/// The poster frame is this far into the video…
const POSTER_FRACTION: f64 = 0.1;
/// …but no later than this (a long video's first scene is usually its title).
const POSTER_MAX_MS: i64 = 10_000;

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Damaged files would otherwise print warnings on stderr; failures come back as errors. (If FFmpeg can't be
        // loaded, opening a file says so.)
        if let (Ok(api), Ok(level)) = (ffi::api(), c_int::try_from(AV_LOG_FATAL)) {
            // SAFETY: av_log_set_level takes a level and sets the process's log level; nothing else is touched.
            unsafe { (api.av_log_set_level)(level) }
        }
    });
}

/// What a video is, as far as the catalog cares.
#[derive(Debug, PartialEq)]
pub struct Probe {
    /// Upright display size (display matrix applied, square pixels).
    pub width: u32,
    pub height: u32,
    pub duration_ms: i64,
    /// The video codec's short name (`h264`, `hevc`, `vp9`, `av1`…).
    pub codec: &'static str,
    /// Frames in the stream (counted by the container, else estimated from the duration and frame rate).
    pub frames: u32,
    /// When it was recorded. Phones' local-time tag gives wall-clock time; the standard `creation_time` is UTC.
    pub taken: Option<Taken>,
    /// EXIF-style orientation (1–8) of the display matrix.
    pub orientation: Orientation,
}

fn open(path: &Path) -> Result<Input> {
    init();
    // FFmpeg takes UTF-8 paths, and one past the 260-character limit only in its verbatim form: `utf8_text` makes
    // that form. A name with an unpaired surrogate has no UTF-8 form, and is refused as such.
    let text = pg_platform::native::utf8_text(path)
        .with_context(|| format!("{} has no UTF-8 form, which FFmpeg needs", path.display()))?;
    Input::open(&text).with_context(|| format!("opening {}", path.display()))
}

/// The orientation (EXIF 1–8) that shows the stream as its display matrix asks. The matrix maps stored to shown
/// pixels; FFmpeg's players apply the same (`-autorotate`).
fn display_orientation(stream: &Stream<'_>) -> Orientation {
    match stream.display_matrix() {
        Some(m) => matrix_orientation(m),
        None => Orientation::UPRIGHT,
    }
}

/// The orientation (EXIF 1–8) of a display matrix (`a b u c d v x y w`, 16.16 fixed point but for u, v and w).
fn matrix_orientation(m: [i32; 9]) -> Orientation {
    let [a, b, _, c, d, ..] = m.map(f64::from);
    // A negative determinant is a mirror. FFmpeg's flip (av_display_matrix_flip) mirrors the shown picture after
    // the rotation, by negating a and c; undo it to read the rotation.
    let mirrored = a * d - b * c < 0.0;
    let (a, c) = if mirrored { (-a, -c) } else { (a, c) };
    let (s0, s1) = (a.hypot(c), b.hypot(d));
    if s0 == 0.0 || s1 == 0.0 {
        return Orientation::UPRIGHT;
    }
    // av_display_rotation_get is the counterclockwise angle; turning clockwise by the opposite shows it upright.
    let clockwise = (b / s1).atan2(a / s0).to_degrees();
    let turns = u8::try_from(whole_i64((clockwise / 90.0).round()).rem_euclid(4)).unwrap_or(u8::MAX);
    // EXIF mirrors the stored picture before the rotation: the same picture with the turn reversed.
    Orientation::from_parts(mirrored, Turns::new(if mirrored { (4 - turns) % 4 } else { turns }))
}

pub fn probe(path: &Path) -> Result<Probe> {
    let input = open(path)?;
    let stream = input.best_video().context("no video stream")?;
    let decoder = Decoder::open(&stream)?;
    let codec = ffi::codec_name(stream.codec_id());
    let (mut w, h) = (decoder.width(), decoder.height());
    if w == 0 || h == 0 {
        bail!("no picture size");
    }
    let sar = decoder.sample_aspect_ratio();
    if sar.num > 0 && sar.den > 0 && sar.num != sar.den {
        w = whole_u32((f64::from(w) * f64::from(sar.num) / f64::from(sar.den)).round());
    }
    let orientation = display_orientation(&stream);
    let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };

    let tb = stream.time_base();
    let in_ms = |t: i64| pg_model::num::signed(t) * f64::from(tb.num) * 1000.0 / f64::from(tb.den.max(1));
    let duration_ms = if stream.duration() > 0 {
        whole_i64(in_ms(stream.duration()))
    } else {
        // The container's, in AV_TIME_BASE (µs).
        input.duration().max(0) / 1000
    };
    let rate = stream.avg_frame_rate();
    let frames = match stream.frames() {
        n if n > 0 => n,
        _ if rate.num > 0 && rate.den > 0 => {
            let frames = pg_model::num::signed(duration_ms) / 1000.0 * f64::from(rate.num) / f64::from(rate.den);
            whole_i64(frames.round())
        }
        _ => 1,
    };

    let tag = |key: &str| input.metadata(key).or_else(|| stream.metadata(key));
    let taken = tag("com.apple.quicktime.creationdate")
        .and_then(|d| parse_iso_date(&d))
        .map(Taken::Wall)
        .or_else(|| tag("creation_time").and_then(|d| parse_iso_date(&d)).map(Taken::Utc))
        // Cameras and muxers without a clock write the container's epoch.
        .filter(|t| t.parts().0 > 0);

    Ok(Probe {
        width,
        height,
        duration_ms,
        codec,
        frames: u32::try_from(frames.clamp(1, i64::from(u32::MAX))).unwrap_or(u32::MAX),
        taken,
        orientation,
    })
}

/// Where the poster frame is in a video of this length.
pub fn poster_ms(duration_ms: i64) -> i64 {
    whole_i64(pg_model::num::signed(duration_ms) * POSTER_FRACTION).clamp(0, POSTER_MAX_MS)
}

/// `v` with its fraction dropped, held to `i64`'s range (NaN reads as 0).
fn whole_i64(v: f64) -> i64 {
    let bound = 2f64.powi(63);
    pg_model::num::f64_to_i64(v.clamp(-bound, bound))
}

/// `v` with its fraction dropped, held to `u32`'s range (NaN reads as 0).
fn whole_u32(v: f64) -> u32 {
    pg_model::num::f64_to_u32(v.clamp(0.0, f64::from(u32::MAX)))
}

/// The frame shown at `at_ms` (the first at or after it; the last one if the video is shorter), upright and
/// turned `turns` quarter turns clockwise further (the catalog rotation), as RGB at its display size, in a new
/// buffer: for a one-off frame (the viewer).
pub fn frame(path: &Path, at_ms: i64, turns: Turns) -> Result<Rgb> {
    let mut out = Rgb::default();
    frame_into(path, at_ms, turns, &mut out, &mut Vec::new())?;
    Ok(out)
}

/// [`frame()`] into a worker's buffers, which keep their capacity from file to file: the pixels go into `out`, and
/// are oriented through `spare` (swapped with `out`'s).
pub fn frame_into(path: &Path, at_ms: i64, turns: Turns, out: &mut Rgb, spare: &mut Vec<u8>) -> Result<()> {
    let mut input = open(path)?;
    let stream = input.best_video().context("no video stream")?;
    let index = stream.index();
    let orientation = display_orientation(&stream).compose(turns);
    let tb = stream.time_base();
    let start = if stream.start_time() == NOPTS_VALUE { 0 } else { stream.start_time() };
    let mut decoder = Decoder::open(&stream)?;
    let ms =
        |t: i64| whole_i64(pg_model::num::signed(t - start) * f64::from(tb.num) * 1000.0 / f64::from(tb.den.max(1)));
    if at_ms > 0 {
        // To the keyframe at or before it (AV_TIME_BASE = µs); decoding goes on from there. A file that can't
        // seek is decoded from the start.
        let _ = input.seek(at_ms * 1000);
    }

    let mut packet = Packet::new()?;
    let mut decoded = Frame::new()?;
    let mut last: Option<Frame> = None;
    let mut found = None;
    'packets: while input.read_packet(&mut packet) {
        if packet.stream_index() != index {
            continue;
        }
        // A damaged packet only loses its frames.
        if !decoder.send_packet(&packet) {
            continue;
        }
        while decoder.receive_frame(&mut decoded) {
            let t = decoded.timestamp().map_or(at_ms, ms);
            let f = std::mem::replace(&mut decoded, Frame::new()?);
            if t >= at_ms {
                found = Some(f);
                break 'packets;
            }
            last = Some(f);
        }
    }
    if found.is_none() {
        decoder.send_eof();
        while decoder.receive_frame(&mut decoded) {
            last = Some(std::mem::replace(&mut decoded, Frame::new()?));
        }
    }
    let f = found.or(last).context("no frame decoded")?;
    to_rgb(f, out)?;
    orient_into(out, orientation, spare);
    Ok(())
}

/// RGB at the display size (non-square pixels made square) into `out`, converted with the frame's own color matrix
/// and range. HD video that doesn't say is taken as BT.709, as players do.
fn to_rgb(mut src: Frame, out: &mut Rgb) -> Result<()> {
    let (w, h) = (src.width(), src.height());
    let sar = src.sample_aspect_ratio();
    let dw = if sar.num > 0 && sar.den > 0 {
        whole_u32((f64::from(w) * f64::from(sar.num) / f64::from(sar.den)).round()).max(1)
    } else {
        w
    };
    if src.color_space() == AVColorSpace_AVCOL_SPC_UNSPECIFIED {
        src.set_color_space(if h >= 720 { AVColorSpace_AVCOL_SPC_BT709 } else { AVColorSpace_AVCOL_SPC_BT470BG });
    }
    if src.color_range() == AVColorRange_AVCOL_RANGE_UNSPECIFIED {
        src.set_color_range(AVColorRange_AVCOL_RANGE_MPEG);
    }
    let mut dst = Frame::video(AVPixelFormat_AV_PIX_FMT_RGB24, dw, h)?;
    dst.set_color_range(AVColorRange_AVCOL_RANGE_JPEG);
    // A fresh scaler, used once on two valid frames (src decoded, dst allocated above with its format and size); the
    // scaler reads src's color properties itself, and is freed on return.
    let mut scaler = Scaler::new()?;
    let ret = scaler.scale(&src, &mut dst);
    if ret < 0 {
        bail!("converting the frame to RGB: {}", ffi::error_text(ret));
    }
    let (stride, data) = dst.packed_plane();
    let row = pg_model::num::index(dw) * 3;
    out.pixels.clear();
    out.pixels.reserve(row * pg_model::num::index(h));
    let mut lines = data.chunks(stride.max(1));
    for _ in 0..h {
        let line = lines.next().and_then(|line| line.get(..row)).context("the RGB frame is smaller than its size")?;
        out.pixels.extend_from_slice(line);
    }
    (out.width, out.height) = (pg_model::num::index(dw), pg_model::num::index(h));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eight matrices ffmpeg writes for `-display_rotation` and `-display_hflip`/`-display_vflip`, with the
    /// orientation that shows the picture as ffmpeg's own autorotate does (checked on a four-colour clip).
    #[test]
    fn display_matrix_orientations() {
        const ONE: i32 = 1 << 16;
        let m = |a: i32, b: i32, c: i32, d: i32| [a * ONE, b * ONE, 0, c * ONE, d * ONE, 0, 0, 0, 1 << 30];
        let cases = [
            (m(1, 0, 0, 1), 1),   // none
            (m(-1, 0, 0, 1), 2),  // hflip
            (m(-1, 0, 0, -1), 3), // 180
            (m(1, 0, 0, -1), 4),  // vflip, also 180 + hflip
            (m(0, 1, 1, 0), 5),   // 270 + hflip (transpose)
            (m(0, 1, -1, 0), 6),  // 270 counterclockwise, i.e. 90 clockwise
            (m(0, -1, -1, 0), 7), // 90 + hflip (transverse)
            (m(0, -1, 1, 0), 8),  // 90 counterclockwise
        ];
        for (matrix, want) in cases {
            assert_eq!(matrix_orientation(matrix).get(), want, "{matrix:?}");
        }
        // Scaled and slightly off (a muxer's rounding) still snaps to the nearest quarter turn.
        assert_eq!(matrix_orientation([2 * ONE, -ONE / 50, 0, 0, -2 * ONE, 0, 0, 0, 1 << 30]).get(), 4);
        assert_eq!(matrix_orientation([0; 9]).get(), 1);
    }
}
