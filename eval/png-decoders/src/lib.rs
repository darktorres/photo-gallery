//! PNG decoders behind one interface, each decoding to packed 8-bit samples on one thread, for the speed and output
//! comparison in docs/PNG_DECODERS.md.
//!
//! - The app's `png`, the fork `pg-codec` decodes with (`Codecs::png`): its scratch kept per decoder, text and ICC
//!   chunks skipped, EXPAND, the app's 512 MiB limit.
//! - `png` 0.18 from crates.io, the fork's upstream: the same options and limit, nothing kept between files.
//! - zune-png 0.5, the pure-Rust decoder of the zune-image family.
//!
//! Every decoder writes into an output buffer that is reused across decodes, as the app reuses its output. The
//! output has the channels the decoder gives after EXPAND: 3 for RGB (and for palettes without transparency), 4 for
//! RGBA (and for palettes or RGB with tRNS).

use std::io::Cursor;

use anyhow::{Result, anyhow, bail};

/// `pg-codec`'s limit (codecs.rs `max_decode_bytes`: the default `Limits`' `max_alloc`, 512 MiB).
pub const LIMIT: usize = 512 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dec {
    Fork,
    Upstream,
    Zune,
}

impl Dec {
    pub const ALL: [Dec; 3] = [Dec::Fork, Dec::Upstream, Dec::Zune];

    pub fn name(self) -> &'static str {
        match self {
            Dec::Fork => "png (app fork)",
            Dec::Upstream => "png 0.18",
            Dec::Zune => "zune-png 0.5",
        }
    }
}

/// The decoders with the state they keep between files.
#[derive(Default)]
pub struct Decoders {
    /// The fork's buffers (its `Scratch`), kept from one file to the next as `Codecs` keeps them.
    fork: png::Scratch,
}

/// Width, height and channels of a decoded image; the samples are in the output buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dims {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
}

impl Decoders {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `bytes` into `out` (resized to the image), as 8-bit samples.
    pub fn decode(&mut self, dec: Dec, bytes: &[u8], out: &mut Vec<u8>) -> Result<Dims> {
        match dec {
            Dec::Fork => self.fork(bytes, out),
            Dec::Upstream => upstream(bytes, out),
            Dec::Zune => zune(bytes, out),
        }
    }

    /// `Codecs::png`'s sequence: the header first, then the options that skip text and ICC, then EXPAND and the
    /// image. The scratch goes back to `self.fork` whatever happens.
    fn fork(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> Result<Dims> {
        let scratch = std::mem::take(&mut self.fork);
        let limits = png::Limits { bytes: LIMIT };
        let mut decoder = png::Decoder::new_with_scratch(Cursor::new(bytes), limits, scratch);
        decoder.set_ignore_text_chunk(true);
        decoder.set_ignore_iccp_chunk(true);
        if let Err(err) = decoder.read_header_info() {
            self.fork = decoder.into_scratch();
            bail!("{err}");
        }
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = match decoder.read_info_or_scratch() {
            Ok(reader) => reader,
            Err((err, scratch)) => {
                self.fork = scratch;
                bail!("{err}");
            }
        };
        let result = fork_frame(&mut reader, out);
        self.fork = reader.into_scratch();
        result
    }
}

fn fork_frame(reader: &mut png::Reader<Cursor<&[u8]>>, out: &mut Vec<u8>) -> Result<Dims> {
    let (color, _depth) = reader.output_color_type();
    let (w, h) = reader.info().size();
    out.resize(output_size(reader.output_buffer_size())?, 0);
    reader.next_frame(out).map_err(|e| anyhow!("{e}"))?;
    Ok(Dims { width: w as usize, height: h as usize, channels: color.samples() })
}

fn output_size(size: Option<usize>) -> Result<usize> {
    size.ok_or_else(|| anyhow!("the output doesn't fit in memory"))
}

/// The crates.io `png` with the same options as the fork, without its scratch: its buffers are allocated each time.
fn upstream(bytes: &[u8], out: &mut Vec<u8>) -> Result<Dims> {
    let mut decoder = png_upstream::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png_upstream::Limits { bytes: LIMIT });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png_upstream::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| anyhow!("{e}"))?;
    let (color, _depth) = reader.output_color_type();
    let (w, h) = reader.info().size();
    out.resize(output_size(reader.output_buffer_size())?, 0);
    reader.next_frame(out).map_err(|e| anyhow!("{e}"))?;
    Ok(Dims { width: w as usize, height: h as usize, channels: color.samples() })
}

/// zune-png hands back its own buffer, so each decode allocates it; the output buffer takes that buffer.
fn zune(bytes: &[u8], out: &mut Vec<u8>) -> Result<Dims> {
    // zune's default caps the height at 16384; the app's decoder takes any size up to its memory limit.
    let options = zune_core::options::DecoderOptions::default().set_max_width(1 << 20).set_max_height(1 << 20);
    let mut decoder = zune_png::PngDecoder::new_with_options(zune_core::bytestream::ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| anyhow!("{e:?}"))?;
    let (w, h) = decoder.dimensions().ok_or_else(|| anyhow!("no dimensions"))?;
    let channels = decoder.colorspace().ok_or_else(|| anyhow!("no colourspace"))?.num_components();
    *out = decoder.decode_raw().map_err(|e| anyhow!("{e:?}"))?;
    Ok(Dims { width: w, height: h, channels })
}
