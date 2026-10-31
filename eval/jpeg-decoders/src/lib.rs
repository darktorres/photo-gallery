//! JPEG decoders behind one interface, each decoding to packed RGB8 on one thread, for the speed and output
//! comparison in docs/PROFILING.md.
//!
//! - libjpeg-turbo as the app calls it (`pg-core` decode.rs): accurate integer IDCT, smooth upsampling.
//! - libjpeg-turbo with `TJPARAM_FASTDCT` and `TJPARAM_FASTUPSAMPLE`: the fast but less accurate options.
//! - zune-jpeg, the `image` crate's JPEG decoder since 0.25 (pure Rust, AVX2). It has no DCT scaling.
//! - jpeg-decoder, the `image` crate's decoder before 0.25 (pure Rust), without its rayon threads.
//! - FFmpeg's MJPEG decoder (`lowres` for the scaled decode), then swscale to RGB, as `pg-core` video.rs does.
//! - libvips's `jpegload` (shrink-on-load for the scaled decode), sequential, one thread, its operation cache off.
//!   It decodes with the system's libjpeg (Ubuntu 26.04: libjpeg-turbo 2.1.5), not the vendored 3.x.
//!
//! Decoder state is reused across images where the API allows it, as the app reuses its libjpeg-turbo handle.

use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};
use ffmpeg_next as ffmpeg;
use turbojpeg::raw;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dec {
    Turbo,
    TurboFast,
    Zune,
    JpegDecoder,
    Ffmpeg,
    Vips,
}

impl Dec {
    pub const ALL: [Dec; 6] = [Dec::Turbo, Dec::TurboFast, Dec::Zune, Dec::JpegDecoder, Dec::Ffmpeg, Dec::Vips];

    pub fn name(self) -> &'static str {
        match self {
            Dec::Turbo => "libjpeg-turbo",
            Dec::TurboFast => "libjpeg-turbo fast",
            Dec::Zune => "zune-jpeg",
            Dec::JpegDecoder => "jpeg-decoder",
            Dec::Ffmpeg => "FFmpeg mjpeg",
            Dec::Vips => "libvips",
        }
    }

    /// Whether it can decode at 1/2, 1/4, 1/8 of the size (the IDCT on fewer coefficients).
    pub fn scales(self) -> bool {
        self != Dec::Zune
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Full size.
    Full,
    /// What `analyze` asks for: the smallest DCT scale keeping the long side ≥ 512 and the short side ≥ 320.
    Analyze,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Analyze => "analyze scale",
        }
    }
}

pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// The scale denominator `pg-core` picks (decode.rs): libjpeg-turbo rounds scaled sizes up.
pub fn denom(mode: Mode, w: usize, h: usize) -> usize {
    match mode {
        Mode::Full => 1,
        Mode::Analyze => {
            let (min_w, min_h) = if w >= h { (512, 320) } else { (320, 512) };
            [8, 4, 2].into_iter().find(|&d| w.div_ceil(d) >= min_w && h.div_ceil(d) >= min_h).unwrap_or(1)
        }
    }
}

/// Width and height from the frame header (SOF), without a decoder.
pub fn dimensions(bytes: &[u8]) -> Result<(usize, usize)> {
    ensure!(bytes.starts_with(&[0xFF, 0xD8]), "not a JPEG");
    let mut i = 2;
    while i + 4 <= bytes.len() {
        ensure!(bytes[i] == 0xFF, "bad marker at {i}");
        let m = bytes[i + 1];
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            ensure!(i + 9 <= bytes.len(), "short SOF");
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as usize;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as usize;
            return Ok((w, h));
        }
        i += 2 + len;
    }
    bail!("no frame header")
}

pub struct Decoders {
    turbo: turbojpeg::Decompressor,
    fast: TurboFast,
    /// One opened MJPEG decoder per `lowres` (0–3), which is fixed when a decoder opens.
    ffmpeg: [Option<ffmpeg::decoder::Video>; 4],
}

impl Decoders {
    pub fn new() -> Result<Decoders> {
        ffmpeg::init()?;
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);
        vips::init()?;
        Ok(Decoders { turbo: turbojpeg::Decompressor::new()?, fast: TurboFast::new()?, ffmpeg: Default::default() })
    }

    pub fn decode(&mut self, dec: Dec, mode: Mode, bytes: &[u8]) -> Result<Rgb> {
        match dec {
            Dec::Turbo => turbo(&mut self.turbo, mode, bytes),
            Dec::TurboFast => self.fast.decode(mode, bytes),
            Dec::Zune => {
                ensure!(mode == Mode::Full, "zune-jpeg has no DCT scaling");
                zune(bytes)
            }
            Dec::JpegDecoder => jpeg_decoder(mode, bytes),
            Dec::Ffmpeg => self.ffmpeg_decode(mode, bytes),
            Dec::Vips => vips::decode(mode, bytes),
        }
    }

    fn ffmpeg_decode(&mut self, mode: Mode, bytes: &[u8]) -> Result<Rgb> {
        let (w, h) = dimensions(bytes)?;
        let lowres = denom(mode, w, h).trailing_zeros() as usize;
        let decoder = match &mut self.ffmpeg[lowres] {
            Some(d) => d,
            slot => slot.insert(open_mjpeg(lowres)?),
        };
        decoder.send_packet(&ffmpeg::Packet::copy(bytes))?;
        let mut frame = ffmpeg::frame::Video::empty();
        if decoder.receive_frame(&mut frame).is_err() {
            // A decoder that holds the frame back until the end of the stream.
            decoder.send_eof()?;
            decoder.receive_frame(&mut frame)?;
            decoder.flush();
        }
        ffmpeg_rgb(frame)
    }
}

fn turbo(d: &mut turbojpeg::Decompressor, mode: Mode, bytes: &[u8]) -> Result<Rgb> {
    let header = d.read_header(bytes)?;
    d.set_scaling_factor(turbojpeg::ScalingFactor::new(1, denom(mode, header.width, header.height)))?;
    let s = header.scaled(d.scaling_factor());
    let mut pixels = vec![0u8; s.width * s.height * 3];
    d.decompress(
        bytes,
        turbojpeg::Image {
            pixels: pixels.as_mut_slice(),
            width: s.width,
            pitch: s.width * 3,
            height: s.height,
            format: turbojpeg::PixelFormat::RGB,
        },
    )?;
    Ok(Rgb { width: s.width, height: s.height, pixels })
}

/// A TurboJPEG 3 handle with the fast IDCT and upsampling, which the `turbojpeg` crate doesn't expose together.
struct TurboFast(raw::tjhandle);

impl TurboFast {
    fn new() -> Result<TurboFast> {
        // SAFETY: tj3Init returns a new handle or null; the parameters are set on that handle only.
        unsafe {
            let h = raw::tj3Init(raw::TJINIT_TJINIT_DECOMPRESS as _);
            ensure!(!h.is_null(), "tj3Init failed");
            let fast = TurboFast(h);
            ensure!(raw::tj3Set(h, raw::TJPARAM_TJPARAM_FASTDCT as _, 1) == 0, "TJPARAM_FASTDCT");
            ensure!(raw::tj3Set(h, raw::TJPARAM_TJPARAM_FASTUPSAMPLE as _, 1) == 0, "TJPARAM_FASTUPSAMPLE");
            Ok(fast)
        }
    }

    fn decode(&mut self, mode: Mode, bytes: &[u8]) -> Result<Rgb> {
        let (w, h) = dimensions(bytes)?;
        let d = denom(mode, w, h);
        let (sw, sh) = (w.div_ceil(d), h.div_ceil(d));
        let mut pixels = vec![0u8; sw * sh * 3];
        // SAFETY: the handle is valid for self's lifetime; the input slice and the output buffer (sw × sh RGB,
        // pitch sw × 3, the size TurboJPEG computes for this scaling factor) outlive the calls.
        unsafe {
            let h = self.0;
            ensure!(raw::tj3DecompressHeader(h, bytes.as_ptr(), bytes.len() as _) == 0, "{}", tj_error(h));
            let factor = raw::tjscalingfactor { num: 1, denom: d as _ };
            ensure!(raw::tj3SetScalingFactor(h, factor) == 0, "{}", tj_error(h));
            let ret = raw::tj3Decompress8(
                h,
                bytes.as_ptr(),
                bytes.len() as _,
                pixels.as_mut_ptr(),
                (sw * 3) as _,
                raw::TJPF_TJPF_RGB as _,
            );
            ensure!(ret == 0, "{}", tj_error(h));
        }
        Ok(Rgb { width: sw, height: sh, pixels })
    }
}

impl Drop for TurboFast {
    fn drop(&mut self) {
        // SAFETY: the handle came from tj3Init and isn't used after this.
        unsafe { raw::tj3Destroy(self.0) }
    }
}

/// SAFETY: `h` must be a valid TurboJPEG handle.
unsafe fn tj_error(h: raw::tjhandle) -> String {
    // SAFETY: tj3GetErrorStr returns a NUL-terminated string owned by the handle.
    unsafe { std::ffi::CStr::from_ptr(raw::tj3GetErrorStr(h)).to_string_lossy().into_owned() }
}

fn zune(bytes: &[u8]) -> Result<Rgb> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = d.decode().map_err(|e| anyhow::anyhow!("zune-jpeg: {e:?}"))?;
    let info = d.info().context("zune-jpeg: no image info")?;
    Ok(Rgb { width: info.width as usize, height: info.height as usize, pixels })
}

fn jpeg_decoder(mode: Mode, bytes: &[u8]) -> Result<Rgb> {
    let mut d = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    d.read_info()?;
    let info = d.info().context("jpeg-decoder: no image info")?;
    ensure!(info.pixel_format == jpeg_decoder::PixelFormat::RGB24, "jpeg-decoder: {:?}", info.pixel_format);
    let (w, h) = (info.width as usize, info.height as usize);
    let dn = denom(mode, w, h);
    let (sw, sh) =
        if dn > 1 { d.scale(w.div_ceil(dn) as u16, h.div_ceil(dn) as u16)? } else { (info.width, info.height) };
    let pixels = d.decode()?;
    Ok(Rgb { width: sw as usize, height: sh as usize, pixels })
}

fn open_mjpeg(lowres: usize) -> Result<ffmpeg::decoder::Video> {
    let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::MJPEG).context("no MJPEG decoder")?;
    let mut ctx = ffmpeg::codec::Context::new_with_codec(codec);
    ctx.set_threading(ffmpeg::threading::Config { kind: ffmpeg::threading::Type::None, count: 1 });
    // SAFETY: the context is fresh and not opened yet; lowres is read when it opens (the MJPEG decoder's maximum
    // is 3).
    unsafe { (*ctx.as_mut_ptr()).lowres = lowres as _ };
    Ok(ctx.decoder().video()?)
}

/// Packed RGB through swscale with the frame's own matrix and range, as `pg-core` video.rs converts.
fn ffmpeg_rgb(mut src: ffmpeg::frame::Video) -> Result<Rgb> {
    use ffmpeg::util::color::{Range, Space};
    let (w, h) = (src.width(), src.height());
    if src.color_space() == Space::Unspecified {
        src.set_color_space(Space::BT470BG); // JPEG's JFIF matrix
    }
    if src.color_range() == Range::Unspecified {
        src.set_color_range(Range::JPEG);
    }
    let mut dst = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGB24, w, h);
    dst.set_color_range(Range::JPEG);
    // SAFETY: a fresh context, used once on two valid frames (src decoded, dst allocated with its format and
    // size) and freed before returning.
    let ret = unsafe {
        let mut ctx = ffmpeg::ffi::sws_alloc_context();
        ensure!(!ctx.is_null(), "sws_alloc_context failed");
        let ret = ffmpeg::ffi::sws_scale_frame(ctx, dst.as_mut_ptr(), src.as_ptr());
        ffmpeg::ffi::sws_free_context(&mut ctx);
        ret
    };
    ensure!(ret >= 0, "swscale: {}", ffmpeg::Error::from(ret));
    let (stride, data) = (dst.stride(0), dst.data(0));
    let row = w as usize * 3;
    let mut pixels = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Ok(Rgb { width: w as usize, height: h as usize, pixels })
}

/// libvips through its C API (the `vips` crate needs a newer libvips than Ubuntu's for its build).
mod vips {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::Once;

    use anyhow::{Result, bail, ensure};

    use super::{Mode, Rgb, denom, dimensions};

    const VIPS_ACCESS_SEQUENTIAL: c_int = 1;

    #[link(name = "vips")]
    unsafe extern "C" {
        fn vips_init(argv0: *const c_char) -> c_int;
        fn vips_concurrency_set(concurrency: c_int);
        fn vips_cache_set_max(max: c_int);
        fn vips_jpegload_buffer(buf: *mut c_void, len: usize, out: *mut *mut c_void, ...) -> c_int;
        fn vips_image_write_to_memory(image: *mut c_void, size: *mut usize) -> *mut c_void;
        fn vips_image_get_width(image: *const c_void) -> c_int;
        fn vips_image_get_height(image: *const c_void) -> c_int;
        fn vips_image_get_bands(image: *const c_void) -> c_int;
        fn vips_error_buffer() -> *const c_char;
        fn vips_error_clear();
    }

    #[link(name = "gobject-2.0")]
    unsafe extern "C" {
        fn g_object_unref(object: *mut c_void);
    }

    #[link(name = "glib-2.0")]
    unsafe extern "C" {
        fn g_free(mem: *mut c_void);
    }

    pub fn init() -> Result<()> {
        static INIT: Once = Once::new();
        let mut ok = true;
        // SAFETY: vips_init runs once per process, before any other libvips call; the settings are global.
        INIT.call_once(|| unsafe {
            ok = vips_init(c"jpeg-decoders".as_ptr()) == 0;
            vips_concurrency_set(1);
            // Without this, decoding the same buffer again is a cache hit.
            vips_cache_set_max(0);
        });
        ensure!(ok, "vips_init: {}", error());
        Ok(())
    }

    pub fn decode(mode: Mode, bytes: &[u8]) -> Result<Rgb> {
        let (w, h) = dimensions(bytes)?;
        let shrink = denom(mode, w, h) as c_int;
        // SAFETY: jpegload_buffer only reads `bytes` (the prototype isn't const) and they outlive the image, which
        // is unreferenced before returning; the optional arguments are name/value pairs ending in NULL. The
        // memory from write_to_memory is g_malloc'd, `size` bytes long, and freed after the copy.
        unsafe {
            let mut img: *mut c_void = std::ptr::null_mut();
            let ret = vips_jpegload_buffer(
                bytes.as_ptr() as *mut c_void,
                bytes.len(),
                &mut img,
                c"shrink".as_ptr(),
                shrink,
                c"access".as_ptr(),
                VIPS_ACCESS_SEQUENTIAL,
                std::ptr::null::<c_char>(),
            );
            if ret != 0 {
                bail!("vips_jpegload_buffer: {}", error());
            }
            let (iw, ih, bands) =
                (vips_image_get_width(img) as usize, vips_image_get_height(img) as usize, vips_image_get_bands(img));
            let mut size = 0usize;
            let mem = vips_image_write_to_memory(img, &mut size);
            g_object_unref(img);
            if mem.is_null() {
                bail!("vips_image_write_to_memory: {}", error());
            }
            let pixels = std::slice::from_raw_parts(mem as *const u8, size).to_vec();
            g_free(mem);
            ensure!(bands == 3 && size == iw * ih * 3, "libvips: {bands} bands, {size} bytes for {iw}×{ih}");
            Ok(Rgb { width: iw, height: ih, pixels })
        }
    }

    fn error() -> String {
        // SAFETY: vips_error_buffer returns libvips's NUL-terminated error text, copied before it's cleared.
        unsafe {
            let s = CStr::from_ptr(vips_error_buffer()).to_string_lossy().into_owned();
            vips_error_clear();
            s
        }
    }
}
