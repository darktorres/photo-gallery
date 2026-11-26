//! Videos, through FFmpeg (our own pinned build, which Qt Multimedia plays them with too): what the import
//! stores about one, and the frame its ThumbHash and thumbnails are made from.
//!
//! "Upright" is as players show it: the stream's display matrix (a phone's rotation tag) applied, then the
//! catalog rotation. Near-duplicate detection doesn't cover videos yet: no pixel hash or PDQ.

mod error;
mod ffi;
mod unopened;

pub use unopened::Unopened;

use pg_model::turns::Turns;
use std::ffi::c_int;
use std::path::Path;
use std::sync::Once;

pub use error::{Error, Result};

use ffi::{
    AV_LOG_FATAL, AVColorRange_AVCOL_RANGE_JPEG, AVColorRange_AVCOL_RANGE_MPEG, AVColorRange_AVCOL_RANGE_UNSPECIFIED,
    AVColorSpace_AVCOL_SPC_BT470BG, AVColorSpace_AVCOL_SPC_BT709, AVColorSpace_AVCOL_SPC_UNSPECIFIED,
    AVPixelFormat_AV_PIX_FMT_RGB24, AVRational, Decoder, Frame, Input, NOPTS_VALUE, Packet, Scaler, Stream,
};

use pg_codec::decode::{MinSize, transform_into};
use pg_codec::pixels::Rgb;
use pg_model::analysis::Taken;
use pg_model::date::parse_iso_date;
use pg_model::orientation::Orientation;

/// The poster frame is this far into the video…
const POSTER_FRACTION: f64 = 0.1;
/// …but no later than this (a long video's first scene is usually its title).
const POSTER_MAX_MS: i64 = 10_000;
/// The error of a stream whose picture has no size, from the two places that read it.
const NO_PICTURE_SIZE: &str = "no picture size";

#[expect(unsafe_code, reason = "FFI: the call that sets FFmpeg's log level, through its function pointer")]
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
/// What a video's first video stream says about it, for the import.
pub struct Probe {
    /// Upright display size (display matrix applied, square pixels).
    pub width: u32,
    /// The upright height, as `width` is.
    pub height: u32,
    /// The duration, in milliseconds.
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
    // The file is read through a handle of its own (ffi.rs): its name is whatever the system holds, however long.
    Input::open(path)
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
    let turns = pg_model::num::f64_to_i64((clockwise / 90.0).round());
    // EXIF mirrors the stored picture before the rotation: the same picture with the turn reversed.
    Orientation::from_parts(mirrored, Turns::of(if mirrored { -turns } else { turns }))
}

/// The width a picture of `width` stored pixels shows at, its pixels made square by the sample aspect ratio `sar` (one
/// that says nothing, 0 or negative, leaves it as stored).
fn display_width(width: u32, sar: AVRational) -> u32 {
    if sar.num > 0 && sar.den > 0 {
        pg_model::num::f64_to_u32((f64::from(width) * f64::from(sar.num) / f64::from(sar.den)).round()).max(1)
    } else {
        width
    }
}

/// `ticks` of a stream's `time_base`, in milliseconds.
fn ticks_ms(ticks: i64, time_base: AVRational) -> f64 {
    pg_model::num::signed(ticks) * f64::from(time_base.num) * 1000.0 / f64::from(time_base.den.max(1))
}

/// The size, duration and codec of the video at `path`, read from its first video stream.
pub fn probe(path: &Path) -> Result<Probe> {
    let input = open(path)?;
    let stream = input.best_video().ok_or_else(|| Error::stream("no video stream"))?;
    // A stream nothing can decode is an error here, as it is when a frame is asked for. Its size and aspect ratio are
    // the stream's parameters (what a decoder opened on it would report), so no decoder is opened for them.
    let codec = ffi::codec_name(stream.codec_id());
    if !stream.has_decoder() {
        return Err(Error::stream(format!("no decoder for {codec}")));
    }
    let (w, h) = stream.picture_size();
    if w == 0 || h == 0 {
        return Err(Error::stream(NO_PICTURE_SIZE));
    }
    let w = display_width(w, stream.sample_aspect_ratio());
    let orientation = display_orientation(&stream);
    let (width, height) = if orientation.swaps_axes() { (h, w) } else { (w, h) };

    let duration_ms = if stream.duration() > 0 {
        pg_model::num::f64_to_i64(ticks_ms(stream.duration(), stream.time_base()))
    } else {
        // The container's, in AV_TIME_BASE (µs).
        input.duration().max(0) / 1000
    };
    let rate = stream.avg_frame_rate();
    let frames = match stream.frames()? {
        0 if rate.num > 0 && rate.den > 0 => {
            let frames = pg_model::num::signed(duration_ms) / 1000.0 * f64::from(rate.num) / f64::from(rate.den);
            pg_model::num::f64_to_i64(frames.round())
        }
        0 => 1,
        n => i64::from(n),
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
    pg_model::num::f64_to_i64(pg_model::num::signed(duration_ms) * POSTER_FRACTION).clamp(0, POSTER_MAX_MS)
}

/// The frame shown at `at_ms` (the first at or after it; the last one if the video is shorter), upright and
/// turned `turns` quarter turns clockwise further (the catalog rotation), as RGB at its display size, in a new
/// buffer: for a one-off frame (the viewer).
pub fn frame(path: &Path, at_ms: i64, turns: Turns) -> Result<Rgb> {
    let mut out = Rgb::default();
    frame_into(path, at_ms, turns, MinSize::Full, &mut out, &mut Vec::new())?;
    Ok(out)
}

/// [`frame()`] into a worker's buffers, which keep their capacity from file to file: the pixels go into `out`, and
/// are oriented through `spare` (swapped with `out`'s). The frame is converted at the least size `min` asks for (a
/// still's decode is sized the same way, `pg_codec::decode`), so the conversion and the scaling are one pass.
pub fn frame_into(
    path: &Path,
    at_ms: i64,
    turns: Turns,
    min: MinSize,
    out: &mut Rgb,
    spare: &mut Vec<u8>,
) -> Result<()> {
    let mut input = open(path)?;
    let stream = input.best_video().ok_or_else(|| Error::stream("no video stream"))?;
    let index = stream.index();
    let orientation = display_orientation(&stream).compose(turns);
    let tb = stream.time_base();
    let start = if stream.start_time() == NOPTS_VALUE { 0 } else { stream.start_time() };
    let mut decoder = Decoder::open(&stream)?;
    if at_ms > 0 {
        // To the keyframe at or before it (AV_TIME_BASE = µs); decoding goes on from there. A file that can't
        // seek is decoded from the start.
        let _ = input.seek(at_ms * 1000);
    }

    let mut picker = Picker { at_ms, start, time_base: tb, found: None, last: None };
    let mut packet = Packet::new()?;
    'packets: while input.read_packet(&mut packet) {
        if packet.stream_index() != index {
            continue;
        }
        // A threaded decoder refuses a packet while its frames wait to be read: they are taken, and the packet is sent
        // again. A packet still refused is damaged, and only loses its frames.
        if !decoder.send_packet(&packet) {
            if picker.take(&mut decoder)? {
                break 'packets;
            }
            decoder.send_packet(&packet);
        }
        if picker.take(&mut decoder)? {
            break;
        }
    }
    if picker.found.is_none() {
        decoder.send_eof();
        picker.take(&mut decoder)?;
    }
    let f = picker.found.or(picker.last).ok_or_else(|| Error::stream("no frame decoded"))?;
    to_rgb(f, orientation, min, out)?;
    if orientation != Orientation::UPRIGHT {
        (out.width, out.height) = transform_into(&out.pixels, out.width, out.height, 3, orientation, spare);
        std::mem::swap(&mut out.pixels, spare);
    }
    Ok(())
}

/// The frames of one decode, as the decoder gives them: the first at or after the target time (`found`), or else the
/// last one before it (`last`).
struct Picker {
    at_ms: i64,
    /// The stream's first timestamp, and its time base: a frame's time is `ticks_ms(t - start, time_base)`.
    start: i64,
    time_base: AVRational,
    found: Option<Frame>,
    last: Option<Frame>,
}

impl Picker {
    /// Takes the decoder's frames until it has no more (or until it has the target). True once it has the target.
    fn take(&mut self, decoder: &mut Decoder) -> Result<bool> {
        let mut decoded = Frame::new()?;
        while decoder.receive_frame(&mut decoded) {
            let t = decoded
                .timestamp()
                .map_or(self.at_ms, |t| pg_model::num::f64_to_i64(ticks_ms(t - self.start, self.time_base)));
            let f = std::mem::replace(&mut decoded, Frame::new()?);
            if t >= self.at_ms {
                self.found = Some(f);
                return Ok(true);
            }
            self.last = Some(f);
        }
        Ok(false)
    }
}

/// The scale a frame is converted at, so that it keeps what `min` asks for: the least scale at which an upright
/// `w × h` picture still meets it, as the still decoder's DCT scale keeps it. At most 1: a frame is never enlarged.
fn scale_for(min: MinSize, w: usize, h: usize) -> f64 {
    let count = pg_model::num::count;
    let (need_w, need_h) = match min {
        MinSize::Full => return 1.0,
        MinSize::Fit(bw, bh) => return MinSize::fit_scale(bw, bh, w, h).min(1.0),
        MinSize::Upright(mw, mh) => (mw, mh),
        MinSize::Sides(long, short) if w >= h => (long, short),
        MinSize::Sides(long, short) => (short, long),
    };
    (count(need_w) / count(w)).max(count(need_h) / count(h)).min(1.0)
}

/// `x` (a size in pixels, at least 1) scaled by `scale`, rounded up, and kept between 1 and `x`.
fn scaled(x: u32, scale: f64) -> u32 {
    pg_model::num::f64_to_u32((f64::from(x) * scale).ceil()).clamp(1, x)
}

/// RGB at the display size (non-square pixels made square) into `out`, converted with the frame's own color matrix
/// and range, and scaled to the size `min` asks for in the same conversion. `orientation` is the frame's, so the
/// scale is that of the upright picture. HD video that doesn't say is taken as BT.709, as players do.
fn to_rgb(mut src: Frame, orientation: Orientation, min: MinSize, out: &mut Rgb) -> Result<()> {
    let (w, h) = (src.width(), src.height());
    let dw = display_width(w, src.sample_aspect_ratio());
    if dw == 0 || h == 0 {
        return Err(Error::stream(NO_PICTURE_SIZE));
    }
    if src.color_space() == AVColorSpace_AVCOL_SPC_UNSPECIFIED {
        src.set_color_space(if h >= 720 { AVColorSpace_AVCOL_SPC_BT709 } else { AVColorSpace_AVCOL_SPC_BT470BG });
    }
    if src.color_range() == AVColorRange_AVCOL_RANGE_UNSPECIFIED {
        src.set_color_range(AVColorRange_AVCOL_RANGE_MPEG);
    }
    let (uw, uh) = if orientation.swaps_axes() { (h, dw) } else { (dw, h) };
    let scale = scale_for(min, pg_model::num::index(uw), pg_model::num::index(uh));
    let (tw, th) = (scaled(dw, scale), scaled(h, scale));
    let mut dst = Frame::video(AVPixelFormat_AV_PIX_FMT_RGB24, tw, th)?;
    dst.set_color_range(AVColorRange_AVCOL_RANGE_JPEG);
    // A fresh scaler, used once on two valid frames (src decoded, dst allocated above with its format and size); the
    // scaler reads src's color properties itself, and is freed on return.
    let mut scaler = Scaler::new()?;
    let ret = scaler.scale(&src, &mut dst);
    if ret < 0 {
        return Err(Error::ffmpeg(format!("converting the frame to RGB: {}", ffi::error_text(ret))));
    }
    let (stride, data) = dst.packed_plane();
    let (row, rows) = (pg_model::num::index(tw) * 3, pg_model::num::index(th));
    out.pixels.clear();
    out.pixels.reserve(row * rows);
    if stride == row {
        // The rows are packed one after another: one copy.
        let packed = data.get(..row * rows).ok_or_else(|| Error::stream("the RGB frame is smaller than its size"))?;
        out.pixels.extend_from_slice(packed);
    } else {
        let mut lines = data.chunks(stride.max(1));
        for _ in 0..rows {
            let line = lines.next().and_then(|line| line.get(..row));
            let line = line.ok_or_else(|| Error::stream("the RGB frame is smaller than its size"))?;
            out.pixels.extend_from_slice(line);
        }
    }
    (out.width, out.height) = (pg_model::num::index(tw), rows);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // The eight matrices ffmpeg writes for `-display_rotation` and `-display_hflip`/`-display_vflip`, with the
    // orientation that shows the picture as ffmpeg's own autorotate does (checked on a four-colour clip).
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

    /// The sample clip (tools/make_test_video.py): 64x48, six frames, H.264.
    const SAMPLE: &[u8] = pg_testkit::VIDEO_SAMPLE;

    /// A fresh folder for one test: a run folder of its own.
    fn scratch_dir(test: &str) -> PathBuf {
        pg_testkit::run_folder("scratch_video", &format!("video-{test}")).unwrap()
    }

    /// The sample under `name`, a name the system holds exactly (on Windows a lone surrogate, on Unix a raw byte: FFmpeg
    /// can't be given either by path).
    fn sample_named(dir: &Path, name: &[u8]) -> PathBuf {
        let path = dir.join(pg_platform::names::os_name(&pg_model::name::Name::from_raw(name)));
        std::fs::write(&path, SAMPLE).unwrap();
        path
    }

    // A video whose name has no UTF-8 form is read as the same video: the probe and a frame (which seeks) agree with
    // the ordinary name's, so the reader's bytes are the file's.
    #[test]
    fn a_video_with_a_name_that_is_not_utf8_is_read_as_its_own_bytes() {
        let dir = scratch_dir("names");
        let plain = sample_named(&dir, b"plain.mp4");
        let odd = sample_named(&dir, if cfg!(windows) { b"odd\xED\xA0\x80.mp4" } else { b"odd\xFF.mp4" });
        let (a, b) = (probe(&plain).unwrap(), probe(&odd).unwrap());
        assert_eq!((a.width, a.height, a.frames), (64, 48, 6));
        assert_eq!((b.width, b.height, b.frames), (a.width, a.height, a.frames));
        let (fa, fb) = (frame(&plain, 500, Turns::NONE).unwrap(), frame(&odd, 500, Turns::NONE).unwrap());
        assert_eq!((fb.width, fb.height), (64, 48));
        assert_eq!(fa.pixels, fb.pixels, "the same frame, read from the same bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A file that is not a video, and a file that isn't there, are errors (the reader's open fails cleanly).
    #[test]
    fn a_file_that_is_not_a_video_or_is_missing_is_an_error() {
        let dir = scratch_dir("errors");
        let junk = dir.join("junk.mp4");
        std::fs::write(&junk, b"not a video at all ".repeat(64)).unwrap();
        assert!(probe(&junk).is_err());
        assert!(probe(&dir.join("missing.mp4")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
