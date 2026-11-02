use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::convert::{TryFrom, TryInto};
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroU64;
use std::io;
use std::io::prelude::*;

use crate::common::{Block, Frame};
use crate::{AnyExtension, Extension, Repeat};

mod converter;
mod decoder;

pub use self::decoder::{
    Decoded, DecodingError, DecodingFormatError, FrameDataType, FrameDecoder, OutputBuffer,
    StreamingDecoder, Version, PLTE_CHANNELS,
};

pub use self::converter::ColorOutput;
use self::converter::PixelConverter;
use self::decoder::StreamBuffers;

/// photo-gallery: what a decoder allocates as it reads a GIF (the LZW decoders and their tables, one per code size,
/// the extension buffer, the palettes, the indices it converts to RGBA, a frame's buffer, XMP and ICC), kept from
/// one image to the next: [`DecodeOptions::read_info_with`], [`Decoder::into_scratch`]. A decoder reads in them
/// exactly as in new ones.
#[derive(Default)]
pub struct Scratch {
    stream: StreamBuffers,
    converter: Vec<u8>,
    frame: Vec<u8>,
    xmp: Vec<u8>,
    icc: Vec<u8>,
}

impl Scratch {
    /// Free the buffers that hold more than `bytes`, after an image far larger than the rest.
    pub fn trim(&mut self, bytes: usize) {
        self.stream.trim(bytes);
        for buf in [
            &mut self.converter,
            &mut self.frame,
            &mut self.xmp,
            &mut self.icc,
        ] {
            if buf.capacity() > bytes {
                *buf = Vec::new();
            }
        }
    }
}

/// photo-gallery: of two buffers, the larger.
fn larger(a: Vec<u8>, b: Vec<u8>) -> Vec<u8> {
    if a.capacity() >= b.capacity() {
        a
    } else {
        b
    }
}

#[derive(Clone, Debug)]
/// The maximum amount of memory the decoder is allowed to use for each frame
pub enum MemoryLimit {
    /// Enforce no memory limit.
    ///
    /// If you intend to process images from unknown origins this is a potentially dangerous
    /// constant to use, as your program could be vulnerable to decompression bombs. That is,
    /// malicious images crafted specifically to require an enormous amount of memory to process
    /// while having a disproportionately small file size.
    ///
    /// The risks for modern machines are a bit smaller as the size of each frame cannot
    /// exceed 16GiB, but this is still a significant amount of memory.
    Unlimited,
    /// Limit the amount of memory that can be used for a single frame to this many bytes.
    ///
    /// It may not be enforced precisely due to allocator overhead
    /// and the decoder potentially allocating small auxiliary buffers,
    /// but it will precisely limit the size of the output buffer for each frame.
    //
    // The `NonZero` type is used to make FFI simpler.
    // Due to the guaranteed niche optimization, `Unlimited` will be represented as `0`,
    // and the whole enum as a simple `u64`.
    Bytes(NonZeroU64),
}

impl MemoryLimit {
    fn check_size(&self, size: usize) -> Result<(), DecodingError> {
        match self {
            Self::Unlimited => Ok(()),
            Self::Bytes(limit) => {
                if size as u64 <= limit.get() {
                    Ok(())
                } else {
                    Err(DecodingError::MemoryLimit)
                }
            }
        }
    }

    fn buffer_size(&self, color: ColorOutput, width: u16, height: u16) -> Option<usize> {
        let pixels = u64::from(width) * u64::from(height);

        let bytes_per_pixel = match color {
            ColorOutput::Indexed => 1,
            ColorOutput::RGBA => 4,
        };

        // This cannot overflow because the maximum possible value is 16GiB, well within u64 range
        let total_bytes = pixels * bytes_per_pixel;

        // On 32-bit platforms the size of the output buffer may not be representable
        let usize_bytes = usize::try_from(total_bytes).ok()?;

        match self {
            Self::Unlimited => Some(usize_bytes),
            Self::Bytes(limit) => {
                if total_bytes > limit.get() {
                    None
                } else {
                    Some(usize_bytes)
                }
            }
        }
    }

    #[inline]
    fn try_reserve(&self, vec: &mut Vec<u8>, additional: usize) -> Result<(), DecodingError> {
        let len = vec
            .len()
            .checked_add(additional)
            .ok_or(DecodingError::MemoryLimit)?;
        self.check_size(len)?;
        vec.try_reserve(additional)
            .map_err(|_| DecodingError::OutOfMemory)?;
        Ok(())
    }
}

/// Options for opening a GIF decoder. [`DecodeOptions::read_info`] will start the decoder.
#[derive(Clone, Debug)]
pub struct DecodeOptions {
    memory_limit: MemoryLimit,
    color_output: ColorOutput,
    check_frame_consistency: bool,
    skip_frame_decoding: bool,
    check_for_end_code: bool,
    allow_unknown_blocks: bool,
    // photo-gallery: see `Self::skip_metadata`.
    skip_metadata: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeOptions {
    /// Creates a new decoder builder
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self {
            memory_limit: MemoryLimit::Bytes(50_000_000.try_into().unwrap()), // 50 MB
            color_output: ColorOutput::Indexed,
            check_frame_consistency: false,
            skip_frame_decoding: false,
            check_for_end_code: false,
            allow_unknown_blocks: false,
            skip_metadata: false,
        }
    }

    /// Configure how color data is decoded.
    #[inline]
    pub fn set_color_output(&mut self, color: ColorOutput) {
        self.color_output = color;
    }

    /// Configure a memory limit for decoding.
    pub fn set_memory_limit(&mut self, limit: MemoryLimit) {
        self.memory_limit = limit;
    }

    /// Configure if frames must be within the screen descriptor.
    ///
    /// The default is `false`.
    ///
    /// When turned on, all frame descriptors being read must fit within the screen descriptor or
    /// otherwise an error is returned and the stream left in an unspecified state.
    ///
    /// When turned off, frames may be arbitrarily larger or offset in relation to the screen. Many
    /// other decoder libraries handle this in highly divergent ways. This moves all checks to the
    /// caller, for example to emulate a specific style.
    pub fn check_frame_consistency(&mut self, check: bool) {
        self.check_frame_consistency = check;
    }

    /// Configure whether to skip decoding frames.
    ///
    /// The default is false.
    ///
    /// When turned on, LZW decoding is skipped. [`Decoder::read_next_frame`] will return
    /// compressed LZW bytes in frame's data.
    /// [`Decoder::next_frame_info`] will return the metadata of the next frame as usual.
    /// This is useful to count frames without incurring the overhead of decoding.
    pub fn skip_frame_decoding(&mut self, skip: bool) {
        self.skip_frame_decoding = skip;
    }

    /// photo-gallery: configure whether to skip the XMP and ICC application extensions.
    ///
    /// The default is false.
    ///
    /// When turned on, they are skipped like unknown application extensions: [`Decoder::xmp_metadata`] and
    /// [`Decoder::icc_profile`] are `None`, and their bytes are neither collected nor counted against the memory
    /// limit. For callers that read a file's metadata themselves.
    pub fn skip_metadata(&mut self, skip: bool) {
        self.skip_metadata = skip;
    }

    /// Configure if LZW encoded blocks must end with a marker end code.
    ///
    /// The default is `false`.
    ///
    /// When turned on, all image data blocks—which are LZW encoded—must contain a special bit
    /// sequence signalling the end of the data. LZW processing terminates when this code is
    /// encountered. The specification states that it must be the last code output by the encoder
    /// for an image.
    ///
    /// When turned off then image data blocks can simply end. Note that this might silently ignore
    /// some bits of the last or second to last byte.
    pub fn check_lzw_end_code(&mut self, check: bool) {
        self.check_for_end_code = check;
    }

    /// Configure if unknown blocks are allowed to be decoded.
    ///
    /// The default is `false`.
    ///
    /// When turned on, the decoder will allow unknown blocks to be in the
    /// `BlockStart` position.
    ///
    /// When turned off, decoded block starts must mark an `Image`, `Extension`,
    /// or `Trailer` block. Otherwise, the decoded image will return an error.
    /// If an unknown block error is returned from decoding, enabling this
    /// setting may allow for a further state of decoding on the next attempt.
    ///
    /// This option also allows unknown extension blocks. The decoder assumes the follow the same
    /// block layout, i.e. a sequence of zero-length terminated sub-blocks immediately follow the
    /// extension introducer.
    pub fn allow_unknown_blocks(&mut self, check: bool) {
        self.allow_unknown_blocks = check;
    }

    /// Reads the logical screen descriptor including the global color palette
    ///
    /// Returns a [`Decoder`]. All decoder configuration has to be done beforehand.
    pub fn read_info<R: Read>(self, r: R) -> Result<Decoder<R>, DecodingError> {
        Decoder::<R>::with_no_init(io::BufReader::new(r), self, Scratch::default()).init()
    }

    /// photo-gallery: [`Self::read_info`] in the buffers of an earlier decoder ([`Decoder::into_scratch`]), which
    /// come back with the error if the headers can't be read. `r` is read as it is, without a `BufReader` around it
    /// (which upstream allocates for each decoder, then copies the file through): it must be buffered itself, a
    /// `Cursor` over a file in memory or a `BufReader`.
    pub fn read_info_with<B: BufRead>(
        self,
        r: B,
        scratch: Scratch,
    ) -> Result<Decoder<B, B>, (DecodingError, Scratch)> {
        let mut decoder = Decoder::with_no_init(r, self, scratch);
        match decoder.read_headers() {
            Ok(()) => Ok(decoder),
            Err(err) => Err((err, decoder.into_scratch())),
        }
    }
}

// photo-gallery: over any buffered reader (upstream: a `BufReader` it made).
struct ReadDecoder<B: BufRead> {
    reader: B,
    decoder: StreamingDecoder,
    at_eof: bool,
}

impl<B: BufRead> ReadDecoder<B> {
    #[inline(never)]
    fn decode_next(
        &mut self,
        write_into: &mut OutputBuffer<'_>,
    ) -> Result<Option<Decoded>, DecodingError> {
        while !self.at_eof {
            let (consumed, result) = {
                let buf = self.reader.fill_buf()?;
                if buf.is_empty() {
                    return Err(DecodingError::UnexpectedEof);
                }

                self.decoder.update(buf, write_into)?
            };
            self.reader.consume(consumed);
            match result {
                Decoded::Nothing => (),
                Decoded::BlockStart(Block::Trailer) => {
                    self.at_eof = true;
                }
                result => return Ok(Some(result)),
            }
        }
        Ok(None)
    }

    fn into_inner(self) -> B {
        self.reader
    }

    fn decode_next_bytes(&mut self, out: &mut OutputBuffer<'_>) -> Result<usize, DecodingError> {
        match self.decode_next(out)? {
            Some(Decoded::BytesDecoded(len)) => Ok(len.get()),
            Some(Decoded::DataEnd) => Ok(0),
            _ => Err(DecodingError::format("unexpected data")),
        }
    }
}
/// Headers for supported extensions.
const EXT_NAME_NETSCAPE: &[u8] = b"NETSCAPE2.0";
const EXT_NAME_XMP: &[u8] = b"XMP DataXMP";
const EXT_NAME_ICC: &[u8] = b"ICCRGBG1012";

/// State when parsing application extension
enum AppExtensionState {
    /// Waiting for app name
    None,
    Netscape,
    Xmp,
    Icc,
    Skip,
}

#[allow(dead_code)]
/// GIF decoder. Create [`DecodeOptions`] to get started, and call [`DecodeOptions::read_info`].
///
/// photo-gallery: `B` is the buffered reader it reads `R` through: a `BufReader` it makes ([`DecodeOptions::read_info`])
/// or, `R` being buffered itself, `R` ([`DecodeOptions::read_info_with`]).
pub struct Decoder<R: Read, B: BufRead = io::BufReader<R>> {
    decoder: ReadDecoder<B>,
    pixel_converter: PixelConverter,
    memory_limit: MemoryLimit,
    bg_color: Option<u8>,
    repeat: Repeat,
    current_frame: Frame<'static>,
    current_frame_data_type: FrameDataType,
    app_extension_state: AppExtensionState,
    /// XMP metadata bytes.
    xmp_metadata: Option<Vec<u8>>,
    /// ICC profile bytes.
    icc_profile: Option<Vec<u8>>,
    /// photo-gallery: the buffer of the frame before, and XMP and ICC vectors, to read the next ones in.
    spare_frame: Vec<u8>,
    spare_xmp: Vec<u8>,
    spare_icc: Vec<u8>,
    /// photo-gallery: `DecodeOptions::skip_metadata`.
    skip_metadata: bool,
    source: PhantomData<R>,
}

impl<R> Decoder<R>
where
    R: Read,
{
    /// Create a new decoder with default options.
    #[inline]
    pub fn new(reader: R) -> Result<Self, DecodingError> {
        DecodeOptions::new().read_info(reader)
    }

    /// Return a builder that allows configuring limits etc.
    #[must_use]
    #[inline]
    pub fn build() -> DecodeOptions {
        DecodeOptions::new()
    }
}

impl<R, B> Decoder<R, B>
where
    R: Read,
    B: BufRead,
{
    // photo-gallery: over `reader` as it is, in the buffers of `scratch`.
    fn with_no_init(reader: B, options: DecodeOptions, scratch: Scratch) -> Self {
        let Scratch {
            stream,
            converter,
            frame,
            xmp,
            icc,
        } = scratch;
        Self {
            decoder: ReadDecoder {
                reader,
                decoder: StreamingDecoder::with_buffers(&options, stream),
                at_eof: false,
            },
            bg_color: None,
            pixel_converter: PixelConverter::with_buffer(options.color_output, converter),
            memory_limit: options.memory_limit.clone(),
            repeat: Repeat::default(),
            current_frame: Frame::default(),
            current_frame_data_type: FrameDataType::Pixels,
            app_extension_state: AppExtensionState::None,
            skip_metadata: options.skip_metadata,
            xmp_metadata: None,
            icc_profile: None,
            spare_frame: frame,
            spare_xmp: xmp,
            spare_icc: icc,
            source: PhantomData,
        }
    }

    /// photo-gallery: the buffers, for [`DecodeOptions::read_info_with`].
    pub fn into_scratch(self) -> Scratch {
        let mut stream = self.decoder.decoder.into_buffers();
        let (converter, global_palette) = self.pixel_converter.into_buffers();
        stream.keep_global_palette(global_palette);
        if let Some(palette) = self.current_frame.palette {
            stream.keep_palette(palette);
        }
        let frame = match self.current_frame.buffer {
            Cow::Owned(buffer) => larger(buffer, self.spare_frame),
            Cow::Borrowed(_) => self.spare_frame,
        };
        Scratch {
            stream,
            converter,
            frame,
            xmp: larger(self.xmp_metadata.unwrap_or_default(), self.spare_xmp),
            icc: larger(self.icc_profile.unwrap_or_default(), self.spare_icc),
        }
    }

    fn init(mut self) -> Result<Self, DecodingError> {
        self.read_headers()?;
        Ok(self)
    }

    // photo-gallery: `init`'s work, the decoder borrowed, so that a failure can give its buffers back.
    fn read_headers(&mut self) -> Result<(), DecodingError> {
        const APP_EXTENSION: AnyExtension = AnyExtension(Extension::Application as u8);
        loop {
            match self.decoder.decode_next(&mut OutputBuffer::None)? {
                Some(Decoded::BackgroundColor(bg_color)) => {
                    self.bg_color = Some(bg_color);
                }
                Some(Decoded::GlobalPalette(palette)) => {
                    self.pixel_converter.set_global_palette(palette);
                }
                Some(Decoded::SubBlock {
                    ext: APP_EXTENSION,
                    is_last,
                }) => {
                    self.read_application_extension(is_last)?;
                }
                Some(Decoded::HeaderEnd) => break,
                Some(_) => {
                    // There will be extra events when parsing application extension
                    continue;
                }
                None => {
                    return Err(DecodingError::format(
                        "file does not contain any image data",
                    ))
                }
            }
        }
        // If the background color is invalid, ignore it
        if let Some(palette) = self.pixel_converter.global_palette() {
            if self.bg_color.unwrap_or(0) as usize >= (palette.len() / PLTE_CHANNELS) {
                self.bg_color = None;
            }
        }
        Ok(())
    }

    fn read_application_extension(&mut self, is_last: bool) -> Result<(), DecodingError> {
        let data = self.decoder.decoder.last_ext_sub_block();
        match self.app_extension_state {
            AppExtensionState::None => {
                // GIF spec requires len == 11
                self.app_extension_state = match data {
                    EXT_NAME_NETSCAPE => AppExtensionState::Netscape,
                    // photo-gallery: in a kept vector, emptied (upstream: `Vec::new`).
                    // photo-gallery: unless asked not to (`DecodeOptions::skip_metadata`).
                    EXT_NAME_XMP if !self.skip_metadata => {
                        let mut xmp = (self.xmp_metadata.take())
                            .unwrap_or_else(|| mem::take(&mut self.spare_xmp));
                        xmp.clear();
                        self.xmp_metadata = Some(xmp);
                        AppExtensionState::Xmp
                    }
                    EXT_NAME_ICC if !self.skip_metadata => {
                        let mut icc = (self.icc_profile.take())
                            .unwrap_or_else(|| mem::take(&mut self.spare_icc));
                        icc.clear();
                        self.icc_profile = Some(icc);
                        AppExtensionState::Icc
                    }
                    _ => AppExtensionState::Skip,
                }
            }
            AppExtensionState::Netscape => {
                if let [1, rest @ ..] = data {
                    if let Ok(repeat) = rest.try_into().map(u16::from_le_bytes) {
                        self.repeat = if repeat == 0 {
                            Repeat::Infinite
                        } else {
                            Repeat::Finite(repeat)
                        };
                    }
                }
                self.app_extension_state = AppExtensionState::Skip;
            }
            AppExtensionState::Xmp => {
                if let Some(xmp_metadata) = &mut self.xmp_metadata {
                    // XMP is not written as a valid "pascal-string", so we need to stitch together
                    // the text from our collected sublock-lengths.
                    self.memory_limit
                        .try_reserve(xmp_metadata, 1 + data.len())?;
                    xmp_metadata.push(data.len() as u8);
                    xmp_metadata.extend_from_slice(data);
                    if is_last {
                        // XMP adds a "ramp" of 257 bytes to the end of the metadata to let the "pascal-strings"
                        // parser converge to the null byte. The ramp looks like "0x01, 0xff, .., 0x01, 0x00".
                        // For convenience and to allow consumers to not be bothered with this implementation detail,
                        // we cut the ramp.
                        const RAMP_SIZE: usize = 257;
                        if xmp_metadata.len() >= RAMP_SIZE
                            && xmp_metadata.ends_with(&[0x03, 0x02, 0x01, 0x00])
                            && xmp_metadata[xmp_metadata.len() - RAMP_SIZE..]
                                .starts_with(&[0x01, 0x0ff])
                        {
                            xmp_metadata.truncate(xmp_metadata.len() - RAMP_SIZE);
                        }
                    }
                }
            }
            AppExtensionState::Icc => {
                if let Some(icc) = &mut self.icc_profile {
                    self.memory_limit.try_reserve(icc, data.len())?;
                    icc.extend_from_slice(data);
                }
            }
            AppExtensionState::Skip => {}
        };
        if is_last {
            self.app_extension_state = AppExtensionState::None;
        }
        Ok(())
    }

    /// Returns the next frame info
    pub fn next_frame_info(&mut self) -> Result<Option<&Frame<'static>>, DecodingError> {
        loop {
            match self.decoder.decode_next(&mut OutputBuffer::None)? {
                Some(Decoded::FrameMetadata(frame_data_type)) => {
                    let frame = self.decoder.decoder.current_frame_mut().take();
                    // photo-gallery: the frame before's palette and buffer kept for the next ones.
                    let before = mem::replace(&mut self.current_frame, frame);
                    if let Some(palette) = before.palette {
                        self.decoder.decoder.keep_palette(palette);
                    }
                    if let Cow::Owned(buffer) = before.buffer {
                        self.spare_frame = larger(buffer, mem::take(&mut self.spare_frame));
                    }
                    self.current_frame_data_type = frame_data_type;
                    if self.current_frame.palette.is_none() && self.global_palette().is_none() {
                        return Err(DecodingError::format(
                            "no color table available for current frame",
                        ));
                    }
                    break;
                }
                Some(_) => (),
                None => return Ok(None),
            }
        }
        Ok(Some(&self.current_frame))
    }

    /// Query information about the frame previously advanced with [`Self::next_frame_info`].
    ///
    /// Returns `None` past the end of file.
    pub fn current_frame_info(&self) -> Option<&Frame<'static>> {
        if self.decoder.at_eof {
            None
        } else {
            Some(&self.current_frame)
        }
    }

    /// Reads the next frame from the image.
    ///
    /// Do not call `Self::next_frame_info` beforehand.
    /// Deinterlaces the result.
    ///
    /// You can also call `.into_iter()` on the decoder to use it as a regular iterator.
    pub fn read_next_frame(&mut self) -> Result<Option<&Frame<'static>>, DecodingError> {
        if self.next_frame_info()?.is_some() {
            // photo-gallery: the frame read in the buffer of the one before, emptied, which both kinds below
            // fill as they would a new one (zeroed to the frame's size, or appended to).
            if let Cow::Borrowed(_) = self.current_frame.buffer {
                let mut buffer = mem::take(&mut self.spare_frame);
                buffer.clear();
                self.current_frame.buffer = Cow::Owned(buffer);
            }
            match self.current_frame_data_type {
                FrameDataType::Pixels => {
                    self.pixel_converter.read_frame(
                        &mut self.current_frame,
                        &mut |out| self.decoder.decode_next_bytes(out),
                        &self.memory_limit,
                    )?;
                }
                FrameDataType::Lzw { min_code_size } => {
                    let mut vec = if matches!(self.current_frame.buffer, Cow::Owned(_)) {
                        let mut vec =
                            mem::replace(&mut self.current_frame.buffer, Cow::Borrowed(&[]))
                                .into_owned();
                        vec.clear();
                        vec
                    } else {
                        Vec::new()
                    };
                    // Guesstimate 2bpp
                    vec.try_reserve(
                        usize::from(self.current_frame.width)
                            * usize::from(self.current_frame.height)
                            / 4,
                    )
                    .map_err(|_| DecodingError::OutOfMemory)?;
                    self.copy_lzw_into_buffer(min_code_size, &mut vec)?;
                    self.current_frame.buffer = Cow::Owned(vec);
                }
            }
            Ok(Some(&self.current_frame))
        } else {
            Ok(None)
        }
    }

    /// This is private for iterator's use
    fn take_current_frame(&mut self) -> Option<Frame<'static>> {
        if self.current_frame.buffer.is_empty() {
            return None;
        }
        Some(self.current_frame.take())
    }

    /// Reads the data of the current frame into a pre-allocated buffer.
    ///
    /// `Self::next_frame_info` needs to be called beforehand.
    /// The length of `buf` must be at least `Self::buffer_size`.
    /// Deinterlaces the result.
    pub fn read_into_buffer(&mut self, buf: &mut [u8]) -> Result<(), DecodingError> {
        self.pixel_converter
            .read_into_buffer(&self.current_frame, buf, &mut |out| {
                self.decoder.decode_next_bytes(out)
            })
    }

    fn copy_lzw_into_buffer(
        &mut self,
        min_code_size: u8,
        buf: &mut Vec<u8>,
    ) -> Result<(), DecodingError> {
        // `write_lzw_pre_encoded_frame` smuggles `min_code_size` in the first byte.
        buf.push(min_code_size);
        loop {
            match self.decoder.decode_next(&mut OutputBuffer::Vec(buf))? {
                Some(Decoded::LzwDataCopied(_len)) => {}
                Some(Decoded::DataEnd) => return Ok(()),
                _ => return Err(DecodingError::format("unexpected data")),
            }
        }
    }

    /// Reads data of the current frame into a pre-allocated buffer until the buffer has been
    /// filled completely.
    ///
    /// The buffer length must be an even number of pixels (multiple of 4 if decoding RGBA).
    ///
    /// `Self::next_frame_info` needs to be called beforehand. Returns `true` if the supplied
    /// buffer could be filled completely. Should not be called after `false` had been returned.
    pub fn fill_buffer(&mut self, buf: &mut [u8]) -> Result<bool, DecodingError> {
        self.pixel_converter
            .fill_buffer(&self.current_frame, buf, &mut |out| {
                self.decoder.decode_next_bytes(out)
            })
    }

    /// Output buffer size
    pub fn buffer_size(&self) -> usize {
        self.pixel_converter
            .buffer_size(&self.current_frame)
            .unwrap()
    }

    /// Line length of the current frame
    pub fn line_length(&self) -> usize {
        self.pixel_converter.line_length(&self.current_frame)
    }

    /// Returns the color palette relevant for the frame that has been decoded
    #[inline]
    pub fn palette(&self) -> Result<&[u8], DecodingError> {
        Ok(match self.current_frame.palette {
            Some(ref table) => table,
            None => self.global_palette().ok_or_else(|| {
                DecodingError::format("no color table available for current frame")
            })?,
        })
    }

    /// The global color palette
    pub fn global_palette(&self) -> Option<&[u8]> {
        self.pixel_converter.global_palette()
    }

    /// Width of the image
    #[inline]
    pub fn width(&self) -> u16 {
        self.decoder.decoder.width()
    }

    /// Height of the image
    #[inline]
    pub fn height(&self) -> u16 {
        self.decoder.decoder.height()
    }

    /// XMP metadata stored in the image.
    #[inline]
    #[must_use]
    pub fn xmp_metadata(&self) -> Option<&[u8]> {
        self.xmp_metadata.as_deref()
    }

    /// ICC profile stored in the image.
    #[inline]
    #[must_use]
    pub fn icc_profile(&self) -> Option<&[u8]> {
        self.icc_profile.as_deref()
    }

    /// Abort decoding and recover the `io::Read` instance
    pub fn into_inner(self) -> B {
        self.decoder.into_inner()
    }

    /// Index of the background color in the global palette
    ///
    /// In practice this is not used, and the background is
    /// always transparent
    pub fn bg_color(&self) -> Option<usize> {
        self.bg_color.map(|v| v as usize)
    }

    /// Number of loop repetitions
    #[inline]
    pub fn repeat(&self) -> Repeat {
        self.repeat
    }
}

impl<R: Read, B: BufRead> IntoIterator for Decoder<R, B> {
    type Item = Result<Frame<'static>, DecodingError>;
    type IntoIter = DecoderIter<R, B>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        DecoderIter {
            inner: self,
            ended: false,
        }
    }
}

/// Use `decoder.into_iter()` to iterate over the frames
pub struct DecoderIter<R: Read, B: BufRead = io::BufReader<R>> {
    inner: Decoder<R, B>,
    ended: bool,
}

impl<R: Read, B: BufRead> DecoderIter<R, B> {
    /// Abort decoding and recover the `io::Read` instance
    ///
    /// Use `for frame in iter.by_ref()` to be able to call this afterwards.
    pub fn into_inner(self) -> B {
        self.inner.into_inner()
    }
}

impl<R: Read, B: BufRead> FusedIterator for DecoderIter<R, B> {}

impl<R: Read, B: BufRead> Iterator for DecoderIter<R, B> {
    type Item = Result<Frame<'static>, DecodingError>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.ended {
            match self.inner.read_next_frame() {
                Ok(Some(_)) => self.inner.take_current_frame().map(Ok),
                Ok(None) => {
                    self.ended = true;
                    None
                }
                Err(err) => {
                    self.ended = true;
                    Some(Err(err))
                }
            }
        } else {
            None
        }
    }
}

/// photo-gallery: a decoder in kept buffers ([`Scratch`]), over a reader given as it is, reads as a new one over
/// its own `BufReader`, whatever it read before.
#[cfg(test)]
mod photo_gallery {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use std::io::{BufReader, Cursor};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// A pseudo-random generator, so that the files are the same from run to run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u8 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u8
        }

        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| self.next()).collect()
        }
    }

    #[derive(Clone)]
    struct FrameSpec {
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        /// The local palette's size in bits (2 to the bits colours).
        local: Option<u8>,
        interlaced: bool,
        transparent: Option<u8>,
        code_size: u8,
        /// Indices are below this (past the palette when larger than it).
        below: u16,
    }

    fn frame(width: u16, height: u16, code_size: u8) -> FrameSpec {
        FrameSpec {
            left: 0,
            top: 0,
            width,
            height,
            local: None,
            interlaced: false,
            transparent: None,
            code_size,
            below: 1 << code_size.min(8),
        }
    }

    fn sub_blocks(out: &mut Vec<u8>, data: &[u8]) {
        for block in data.chunks(255) {
            out.push(block.len() as u8);
            out.extend_from_slice(block);
        }
        out.push(0);
    }

    fn application(name: &[u8; 11], data: &[u8]) -> Vec<u8> {
        let mut ext = vec![0x21, 0xff, 11];
        ext.extend_from_slice(name);
        sub_blocks(&mut ext, data);
        ext
    }

    /// XMP as GIFs hold it: the text as it is, then the 258-byte "magic trailer", then a terminator.
    fn xmp(text: &[u8]) -> Vec<u8> {
        let mut ext = vec![0x21, 0xff, 11];
        ext.extend_from_slice(b"XMP DataXMP");
        ext.extend_from_slice(text);
        ext.push(1);
        ext.extend((0..=255u8).rev());
        ext.push(0);
        ext
    }

    fn gif(
        rng: &mut Rng,
        version: &[u8; 6],
        (width, height): (u16, u16),
        global: Option<u8>,
        background: u8,
        extensions: &[Vec<u8>],
        frames: &[FrameSpec],
    ) -> Vec<u8> {
        let mut out = version.to_vec();
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.push(global.map_or(0, |bits| 0x80 | (bits - 1)));
        out.push(background);
        out.push(0);
        if let Some(bits) = global {
            out.extend(rng.bytes(3 << bits));
        }
        for ext in extensions {
            out.extend_from_slice(ext);
        }
        for f in frames {
            if let Some(index) = f.transparent {
                out.extend([0x21, 0xf9, 4, 0b1001, 7, 0, index, 0]);
            }
            out.push(0x2c);
            for v in [f.left, f.top, f.width, f.height] {
                out.extend(v.to_le_bytes());
            }
            let flags =
                f.local.map_or(0, |bits| 0x80 | (bits - 1)) | if f.interlaced { 0x40 } else { 0 };
            out.push(flags);
            if let Some(bits) = f.local {
                out.extend(rng.bytes(3 << bits));
            }
            let pixels = usize::from(f.width) * usize::from(f.height);
            let indices: Vec<u8> = (0..pixels)
                .map(|_| (u16::from(rng.next()) % f.below.min(256)) as u8)
                .collect();
            let data = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, f.code_size)
                .encode(&indices)
                .unwrap();
            out.push(f.code_size);
            sub_blocks(&mut out, &data);
        }
        out.push(0x3b);
        out
    }

    fn files() -> Vec<(String, Vec<u8>)> {
        let rng = &mut Rng(7);
        let mut files = Vec::new();
        let mut add = |name: &str, bytes: Vec<u8>| files.push((String::from(name), bytes));

        add(
            "one pixel",
            gif(rng, b"GIF89a", (1, 1), Some(1), 0, &[], &[frame(1, 1, 2)]),
        );
        add(
            "256 colours",
            gif(
                rng,
                b"GIF89a",
                (37, 23),
                Some(8),
                3,
                &[],
                &[frame(37, 23, 8)],
            ),
        );
        let mut offset = frame(20, 11, 2);
        (offset.left, offset.top, offset.local, offset.transparent) = (9, 5, Some(2), Some(1));
        let mut interlaced = frame(64, 48, 4);
        interlaced.interlaced = true;
        let mut outside = frame(40, 30, 4);
        (outside.left, outside.top) = (50, 40);
        add(
            "frames",
            gif(
                rng,
                b"GIF89a",
                (64, 48),
                Some(4),
                1,
                &[],
                &[frame(64, 48, 4), offset, interlaced, outside],
            ),
        );
        let mut two = frame(30, 20, 2);
        two.local = Some(1);
        let mut many = frame(33, 17, 7);
        many.local = Some(7);
        add(
            "local palettes",
            gif(
                rng,
                b"GIF89a",
                (40, 30),
                None,
                0,
                &[],
                &[two, many.clone(), many],
            ),
        );
        add(
            "no palette",
            gif(rng, b"GIF89a", (10, 10), None, 0, &[], &[frame(10, 10, 2)]),
        );
        let sizes: Vec<FrameSpec> = (2..=11).map(|size| frame(29, 13, size)).collect();
        add(
            "every code size",
            gif(rng, b"GIF89a", (29, 13), Some(8), 0, &[], &sizes),
        );
        let mut reversed = sizes.clone();
        reversed.reverse();
        add(
            "every code size, reversed",
            gif(rng, b"GIF89a", (29, 13), Some(8), 0, &[], &reversed),
        );
        let text = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF/></x:xmpmeta>".repeat(9);
        let metadata = [
            application(b"NETSCAPE2.0", &[1, 3, 0]),
            vec![0x21, 0xfe, 5, b'h', b'e', b'l', b'l', b'o', 0],
            xmp(&text),
            application(b"ICCRGBG1012", &rng.bytes(700)),
            application(b"SOMETHING12", &rng.bytes(40)),
        ];
        add(
            "metadata",
            gif(
                rng,
                b"GIF89a",
                (16, 16),
                Some(3),
                2,
                &metadata,
                &[frame(16, 16, 3)],
            ),
        );
        let twice = [
            xmp(b"first"),
            xmp(&text),
            application(b"ICCRGBG1012", &rng.bytes(20)),
        ];
        add(
            "two packets",
            gif(
                rng,
                b"GIF89a",
                (16, 16),
                Some(3),
                2,
                &twice,
                &[frame(16, 16, 3)],
            ),
        );
        add(
            "empty frame",
            gif(
                rng,
                b"GIF89a",
                (8, 8),
                Some(2),
                0,
                &[],
                &[frame(0, 0, 2), frame(8, 8, 2)],
            ),
        );
        add(
            "background past the palette",
            gif(rng, b"GIF89a", (8, 8), Some(1), 9, &[], &[frame(8, 8, 2)]),
        );
        let unknown = [vec![0x21, 0x99, 2, 1, 2, 0]];
        add(
            "unknown extension",
            gif(
                rng,
                b"GIF89a",
                (8, 8),
                Some(1),
                0,
                &unknown,
                &[frame(8, 8, 2)],
            ),
        );
        add(
            "87a",
            gif(rng, b"GIF87a", (12, 9), Some(5), 0, &[], &[frame(12, 9, 5)]),
        );
        let mut past = frame(24, 24, 6);
        past.below = 64;
        add(
            "indices past the palette",
            gif(rng, b"GIF89a", (24, 24), Some(4), 0, &[], &[past]),
        );
        let mut large = frame(300, 200, 8);
        large.interlaced = true;
        add(
            "large",
            gif(
                rng,
                b"GIF89a",
                (300, 200),
                Some(8),
                0,
                &[],
                &[large, frame(300, 200, 8)],
            ),
        );

        // Damaged copies: cut short, and with bytes changed.
        let mut damaged = Vec::new();
        for (name, bytes) in &files {
            let len = bytes.len();
            for cut in [7, 13, len / 3, len / 2, len - 2] {
                if cut < len {
                    damaged.push((format!("{name} cut at {cut}"), bytes[..cut].to_vec()));
                }
            }
            for (at, xor) in [
                (10, 0x07),
                (len / 3, 0x5a),
                (len / 2 + 1, 0xff),
                (len - 3, 0x80),
            ] {
                let mut bytes = bytes.clone();
                bytes[at] ^= xor;
                damaged.push((format!("{name} changed at {at}"), bytes));
            }
        }
        files.extend(damaged);
        files
    }

    /// How the decoder is configured and read.
    #[derive(Clone, Copy, Debug)]
    enum Mode {
        /// As the app reads it: RGBA, `next_frame_info` and `read_into_buffer`.
        Into,
        /// `read_next_frame`, RGBA.
        NextRgba,
        /// `read_next_frame`, indices.
        NextIndexed,
        /// `read_next_frame`, LZW data kept.
        Lzw,
        /// `read_next_frame`, with each other option set and a small memory limit.
        Strict,
    }

    const MODES: [Mode; 5] = [
        Mode::Into,
        Mode::NextRgba,
        Mode::NextIndexed,
        Mode::Lzw,
        Mode::Strict,
    ];

    fn options(mode: Mode) -> DecodeOptions {
        let mut options = DecodeOptions::new();
        match mode {
            Mode::Into | Mode::NextRgba => options.set_color_output(ColorOutput::RGBA),
            Mode::NextIndexed => {}
            Mode::Lzw => options.skip_frame_decoding(true),
            Mode::Strict => {
                options.set_color_output(ColorOutput::RGBA);
                options.check_frame_consistency(true);
                options.check_lzw_end_code(true);
                options.allow_unknown_blocks(true);
                options.set_memory_limit(MemoryLimit::Bytes(5000.try_into().unwrap()));
            }
        }
        options
    }

    /// Everything a decoder gives, in order.
    fn read<R: Read, B: BufRead>(decoder: &mut Decoder<R, B>, mode: Mode) -> Vec<String> {
        let mut log = vec![format!(
            "{}×{} background {:?} repeat {:?} palette {:?}",
            decoder.width(),
            decoder.height(),
            decoder.bg_color(),
            decoder.repeat(),
            decoder.global_palette()
        )];
        for _ in 0..16 {
            if let Mode::Into = mode {
                match decoder.next_frame_info() {
                    Ok(Some(frame)) => log.push(format!("{frame:?}")),
                    Ok(None) => break,
                    Err(err) => {
                        log.push(format!("{err:?}"));
                        break;
                    }
                }
                let size = decoder.buffer_size();
                if size > 1 << 22 {
                    log.push(format!("{size} bytes"));
                    break;
                }
                let mut buf = vec![0; size];
                let result = decoder.read_into_buffer(&mut buf);
                log.push(format!("{result:?} {buf:?}"));
                if result.is_err() {
                    break;
                }
            } else {
                match decoder.read_next_frame() {
                    Ok(Some(frame)) => log.push(format!("{frame:?}")),
                    Ok(None) => break,
                    Err(err) => {
                        log.push(format!("{err:?}"));
                        break;
                    }
                }
            }
            log.push(format!(
                "{:?} {:?}",
                decoder.palette(),
                decoder.current_frame_info()
            ));
        }
        log.push(format!(
            "{:?} {:?}",
            decoder.xmp_metadata(),
            decoder.icc_profile()
        ));
        log
    }

    /// Upstream's way: a decoder over a new `BufReader`.
    fn reference(bytes: &[u8], mode: Mode) -> Vec<String> {
        catch_unwind(|| match options(mode).read_info(Cursor::new(bytes)) {
            Ok(mut decoder) => read(&mut decoder, mode),
            Err(err) => vec![format!("{err:?}")],
        })
        .unwrap_or_else(|_| vec![String::from("panicked")])
    }

    fn kept<B: BufRead>(reader: B, mode: Mode, scratch: Scratch) -> (Vec<String>, Scratch) {
        catch_unwind(AssertUnwindSafe(|| {
            match options(mode).read_info_with(reader, scratch) {
                Ok(mut decoder) => {
                    let log = read(&mut decoder, mode);
                    (log, decoder.into_scratch())
                }
                Err((err, scratch)) => (vec![format!("{err:?}")], scratch),
            }
        }))
        .unwrap_or_else(|_| (vec![String::from("panicked")], Scratch::default()))
    }

    fn first_difference(log: &[String], expected: &[String]) -> String {
        let line = (0..log.len().max(expected.len()))
            .find(|&i| log.get(i) != expected.get(i))
            .unwrap_or(0);
        let cut = |l: Option<&String>| l.map(|l| l.chars().take(300).collect::<String>());
        format!(
            "line {line}: {:?} against {:?}",
            cut(log.get(line)),
            cut(expected.get(line))
        )
    }

    /// A log without the pixels a failed read left in its buffer: how far a read gets before it fails depends on
    /// where its input is cut, in upstream's decoder too.
    fn without_partial_pixels(log: &[String]) -> Vec<String> {
        let cut = |line: &String| match line.split_once(" [") {
            Some((result, _)) if line.starts_with("Err(") => String::from(result),
            _ => line.clone(),
        };
        log.iter().map(cut).collect()
    }

    #[test]
    fn kept_buffers_read_as_new_ones() {
        let files = files();
        let cases: Vec<(usize, Mode)> = (0..files.len())
            .flat_map(|i| MODES.iter().map(move |&mode| (i, mode)))
            .collect();
        let chunks = |bytes: &[u8]| BufReader::with_capacity(5, Cursor::new(bytes.to_vec()));
        let mut upstream = Vec::new();
        let (mut direct, mut chunked) = (Vec::new(), Vec::new());
        for &(i, mode) in &cases {
            let bytes = files[i].1.as_slice();
            upstream.push(reference(bytes, mode));
            direct.push(kept(Cursor::new(bytes), mode, Scratch::default()).0);
            chunked.push(kept(chunks(bytes), mode, Scratch::default()).0);
        }

        // Files read whole, fail, and fail late: these are no vacuous comparisons.
        let errors = upstream
            .iter()
            .filter(|log| log.iter().any(|l| l.contains("Err") || l.contains("Error")))
            .count();
        let whole = upstream
            .iter()
            .filter(|log| log.len() > 4 && !log.iter().any(|l| l.contains("Err")))
            .count();
        let first = upstream.iter().filter(|log| log.len() == 1).count();
        assert!(
            whole > 150 && errors > 150 && first > 20,
            "{whole} {errors} {first}"
        );

        // A new decoder over the file as it is reads as upstream's over its `BufReader`, and in 5-byte pieces too,
        // but where the end code is checked (whether a missing one is found depends on where the input is cut).
        let (mut partial_direct, mut partial_chunked, mut end_code) = (0, 0, 0);
        for (c, &(i, mode)) in cases.iter().enumerate() {
            let name = &files[i].0;
            let expected = without_partial_pixels(&upstream[c]);
            partial_direct += usize::from(direct[c] != upstream[c]);
            partial_chunked += usize::from(chunked[c] != upstream[c]);
            let log = without_partial_pixels(&direct[c]);
            assert!(
                log == expected,
                "{name}, {mode:?}: {}",
                first_difference(&log, &expected)
            );
            let log = without_partial_pixels(&chunked[c]);
            if log != expected {
                assert!(
                    matches!(mode, Mode::Strict),
                    "{name}, {mode:?} in 5-byte reads: {}",
                    first_difference(&log, &expected)
                );
                end_code += 1;
            }
        }
        std::println!(
            "{} files, {} cases: {whole} read without errors, {errors} with, {first} failing in the headers; \
             partial pixels otherwise: {partial_direct} over the file, {partial_chunked} in 5-byte pieces; \
             end codes otherwise: {end_code}",
            files.len(),
            cases.len(),
        );

        // A decoder in kept buffers reads exactly as a new one over the same reader, partial pixels included.
        let order: Vec<usize> = (0..cases.len()).chain((0..cases.len()).rev()).collect();
        for trim in [None, Some(0), Some(600)] {
            let (mut over_file, mut over_chunks) = (Scratch::default(), Scratch::default());
            for &c in &order {
                let (i, mode) = cases[c];
                let (name, bytes) = &files[i];
                let (log, scratch) = kept(Cursor::new(bytes.as_slice()), mode, over_file);
                assert!(
                    log == direct[c],
                    "{name}, {mode:?}, trimmed at {trim:?}: {}",
                    first_difference(&log, &direct[c])
                );
                over_file = scratch;
                let (log, scratch) = kept(chunks(bytes), mode, over_chunks);
                assert!(
                    log == chunked[c],
                    "{name}, {mode:?} in 5-byte reads, trimmed at {trim:?}: {}",
                    first_difference(&log, &chunked[c])
                );
                over_chunks = scratch;
                if let Some(bytes) = trim {
                    over_file.trim(bytes);
                    over_chunks.trim(bytes);
                }
            }
        }
    }
}
