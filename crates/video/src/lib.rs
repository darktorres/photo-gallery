//! Videos, through FFmpeg (our own pinned build, which Qt Multimedia plays them with too: crates/README.md
//! "Build"): what the import stores about one, and the frame its ThumbHash and thumbnails are made from.
//!
//! "Upright" is as players show it: the stream's display matrix (a phone's rotation tag) applied, then the
//! catalog rotation. Near-duplicate detection doesn't cover videos yet (PLAN, later list): no pixel hash or PDQ.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::io::Read;
use std::path::Path;
use std::sync::Once;

use anyhow::{Context, Result, bail};
use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::util::frame;
use ffmpeg_next as ffmpeg;

use pg_codec::decode::orient_into;
use pg_codec::pixels::Rgb;
use pg_model::analysis::Taken;
use pg_model::date::parse_iso_date;
use pg_model::orientation;

/// The poster frame is this far into the video…
const POSTER_FRACTION: f64 = 0.1;
/// …but no later than this (a long video's first scene is usually its title).
const POSTER_MAX_MS: i64 = 10_000;

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = ffmpeg::init();
        // Damaged files would otherwise print warnings on stderr; failures come back as errors.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);
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
    pub orientation: u8,
}

fn open(path: &Path) -> Result<ffmpeg::format::context::Input> {
    init();
    ffmpeg::format::input(path).with_context(|| format!("opening {}", path.display()))
}

/// The orientation (EXIF 1–8) that shows the stream as its display matrix asks. The matrix maps stored to shown
/// pixels; FFmpeg's players apply the same (`-autorotate`).
fn display_orientation(stream: &ffmpeg::format::stream::Stream) -> u8 {
    let Some(sd) = stream.side_data().find(|sd| sd.kind() == ffmpeg::codec::packet::side_data::Type::DisplayMatrix)
    else {
        return 1;
    };
    match sd.data().as_chunks::<4>().0.get(..9) {
        Some(m) => matrix_orientation(std::array::from_fn(|i| i32::from_le_bytes(m[i]))),
        None => 1,
    }
}

/// The orientation (EXIF 1–8) of a display matrix (`a b u c d v x y w`, 16.16 fixed point but for u, v and w).
fn matrix_orientation(m: [i32; 9]) -> u8 {
    let [a, b, _, c, d, ..] = m.map(f64::from);
    // A negative determinant is a mirror. FFmpeg's flip (av_display_matrix_flip) mirrors the shown picture after
    // the rotation, by negating a and c; undo it to read the rotation.
    let mirrored = a * d - b * c < 0.0;
    let (a, c) = if mirrored { (-a, -c) } else { (a, c) };
    let (s0, s1) = (a.hypot(c), b.hypot(d));
    if s0 == 0.0 || s1 == 0.0 {
        return 1;
    }
    // av_display_rotation_get is the counterclockwise angle; turning clockwise by the opposite shows it upright.
    let clockwise = (b / s1).atan2(a / s0).to_degrees();
    let turns = ((clockwise / 90.0).round() as i64).rem_euclid(4) as u8;
    // EXIF mirrors the stored picture before the rotation: the same picture with the turn reversed.
    orientation::from_parts(mirrored, if mirrored { (4 - turns) % 4 } else { turns })
}

pub fn probe(path: &Path) -> Result<Probe> {
    let input = open(path)?;
    let stream = input.streams().best(Type::Video).context("no video stream")?;
    let params = stream.parameters();
    let decoder = ffmpeg::codec::context::Context::from_parameters(params.clone())?.decoder().video()?;
    let codec = params.id().name();
    let (mut w, h) = (decoder.width(), decoder.height());
    if w == 0 || h == 0 {
        bail!("no picture size");
    }
    let sar = decoder.aspect_ratio();
    if sar.numerator() > 0 && sar.denominator() > 0 && sar.numerator() != sar.denominator() {
        w = (w as f64 * sar.numerator() as f64 / sar.denominator() as f64).round() as u32;
    }
    let orientation = display_orientation(&stream);
    let (width, height) = if orientation::swaps_axes(orientation) { (h, w) } else { (w, h) };

    let tb = stream.time_base();
    let in_ms = |t: i64| t as f64 * tb.numerator() as f64 * 1000.0 / tb.denominator().max(1) as f64;
    let duration_ms = if stream.duration() > 0 {
        in_ms(stream.duration()) as i64
    } else {
        // The container's, in AV_TIME_BASE (µs).
        input.duration().max(0) / 1000
    };
    let rate = stream.avg_frame_rate();
    let frames = match stream.frames() {
        n if n > 0 => n,
        _ if rate.numerator() > 0 && rate.denominator() > 0 => {
            (duration_ms as f64 / 1000.0 * rate.numerator() as f64 / rate.denominator() as f64).round() as i64
        }
        _ => 1,
    };

    let tag = |key: &str| {
        input.metadata().get(key).map(str::to_string).or_else(|| stream.metadata().get(key).map(str::to_string))
    };
    let taken = tag("com.apple.quicktime.creationdate")
        .and_then(|d| parse_iso_date(&d))
        .map(Taken::Wall)
        .or_else(|| tag("creation_time").and_then(|d| parse_iso_date(&d)).map(Taken::Utc))
        // Cameras and muxers without a clock write the container's epoch.
        .filter(|t| t.parts().0 > 0);

    Ok(Probe { width, height, duration_ms, codec, frames: frames.clamp(1, u32::MAX as i64) as u32, taken, orientation })
}

/// Where the poster frame is in a video of this length.
pub fn poster_ms(duration_ms: i64) -> i64 {
    ((duration_ms as f64 * POSTER_FRACTION) as i64).clamp(0, POSTER_MAX_MS)
}

/// The frame shown at `at_ms` (the first at or after it; the last one if the video is shorter), upright and
/// turned `turns` quarter turns clockwise further (the catalog rotation), as RGB at its display size, in a new
/// buffer: for a one-off frame (the viewer).
pub fn frame(path: &Path, at_ms: i64, turns: u8) -> Result<Rgb> {
    let mut out = Rgb::default();
    frame_into(path, at_ms, turns, &mut out, &mut Vec::new())?;
    Ok(out)
}

/// [`frame()`] into a worker's buffers, which keep their capacity from file to file: the pixels go into `out`, and
/// are oriented through `spare` (swapped with `out`'s).
pub fn frame_into(path: &Path, at_ms: i64, turns: u8, out: &mut Rgb, spare: &mut Vec<u8>) -> Result<()> {
    let mut input = open(path)?;
    let stream = input.streams().best(Type::Video).context("no video stream")?;
    let index = stream.index();
    let orientation = orientation::compose(display_orientation(&stream), turns);
    let tb = stream.time_base();
    let start = if stream.start_time() == ffmpeg::ffi::AV_NOPTS_VALUE { 0 } else { stream.start_time() };
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
    let ms = |t: i64| ((t - start) as f64 * tb.numerator() as f64 * 1000.0 / tb.denominator().max(1) as f64) as i64;
    if at_ms > 0 {
        // To the keyframe at or before it (AV_TIME_BASE = µs); decoding goes on from there. A file that can't
        // seek is decoded from the start.
        let _ = input.seek(at_ms * 1000, ..at_ms * 1000);
    }

    let mut decoded = frame::Video::empty();
    let mut last: Option<frame::Video> = None;
    let mut found = None;
    'packets: for (s, packet) in input.packets() {
        if s.index() != index {
            continue;
        }
        // A damaged packet only loses its frames.
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut decoded).is_ok() {
            let t = decoded.timestamp().map_or(at_ms, ms);
            let f = std::mem::replace(&mut decoded, frame::Video::empty());
            if t >= at_ms {
                found = Some(f);
                break 'packets;
            }
            last = Some(f);
        }
    }
    if found.is_none() {
        let _ = decoder.send_eof();
        while decoder.receive_frame(&mut decoded).is_ok() {
            last = Some(std::mem::replace(&mut decoded, frame::Video::empty()));
        }
    }
    let f = found.or(last).context("no frame decoded")?;
    to_rgb(f, out)?;
    orient_into(out, orientation, spare);
    Ok(())
}

/// RGB at the display size (non-square pixels made square) into `out`, converted with the frame's own color matrix
/// and range. HD video that doesn't say is taken as BT.709, as players do.
fn to_rgb(mut src: frame::Video, out: &mut Rgb) -> Result<()> {
    use ffmpeg::util::color::{Range, Space};
    let (w, h) = (src.width(), src.height());
    let sar = src.aspect_ratio();
    let dw = if sar.numerator() > 0 && sar.denominator() > 0 {
        ((w as f64 * sar.numerator() as f64 / sar.denominator() as f64).round() as u32).max(1)
    } else {
        w
    };
    if src.color_space() == Space::Unspecified {
        src.set_color_space(if h >= 720 { Space::BT709 } else { Space::BT470BG });
    }
    if src.color_range() == Range::Unspecified {
        src.set_color_range(Range::MPEG);
    }
    let mut dst = frame::Video::new(Pixel::RGB24, dw, h);
    dst.set_color_range(Range::JPEG);
    // SAFETY: a fresh context is allocated, used once on two valid frames (src decoded, dst allocated above with
    // its format and size) and freed before returning; sws_scale_frame reads src's color properties itself.
    let ret = unsafe {
        let mut ctx = ffmpeg::ffi::sws_alloc_context();
        if ctx.is_null() {
            bail!("sws_alloc_context failed");
        }
        let ret = ffmpeg::ffi::sws_scale_frame(ctx, dst.as_mut_ptr(), src.as_ptr());
        ffmpeg::ffi::sws_free_context(&mut ctx);
        ret
    };
    if ret < 0 {
        bail!("converting the frame to RGB: {}", ffmpeg::Error::from(ret));
    }
    let (stride, data) = (dst.stride(0), dst.data(0));
    let row = dw as usize * 3;
    out.pixels.clear();
    out.pixels.reserve(row * h as usize);
    for y in 0..h as usize {
        out.pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    (out.width, out.height) = (dw as usize, h as usize);
    Ok(())
}

/// BLAKE3 of a file, streamed (videos aren't read into memory whole).
pub fn file_hash(path: &Path) -> Result<[u8; 32]> {
    let mut file = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
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
            assert_eq!(matrix_orientation(matrix), want, "{matrix:?}");
        }
        // Scaled and slightly off (a muxer's rounding) still snaps to the nearest quarter turn.
        assert_eq!(matrix_orientation([2 * ONE, -ONE / 50, 0, 0, -2 * ONE, 0, 0, 0, 1 << 30]), 4);
        assert_eq!(matrix_orientation([0; 9]), 1);
    }
}
