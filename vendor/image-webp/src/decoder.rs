use byteorder_lite::{LittleEndian, ReadBytesExt};
use quick_error::quick_error;

use std::collections::HashMap;
use std::io::{self, BufRead, Cursor, Read, Seek};
use std::num::NonZeroU16;
use std::ops::Range;

use crate::extended::{self, get_alpha_predictor, read_alpha_chunk, WebPExtendedInfo};

use super::lossless::{LosslessDecoder, LosslessScratch};
use super::vp8::{Vp8Decoder, Vp8Scratch};

quick_error! {
    /// Errors that can occur when attempting to decode a WebP image
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum DecodingError {
        /// An IO error occurred while reading the file
        IoError(err: io::Error) {
            from()
            display("IO Error: {}", err)
            source(err)
        }

        /// RIFF's "RIFF" signature not found or invalid
        RiffSignatureInvalid(err: [u8; 4]) {
            display("Invalid RIFF signature: {err:x?}")
        }

        /// WebP's "WEBP" signature not found or invalid
        WebpSignatureInvalid(err: [u8; 4]) {
            display("Invalid WebP signature: {err:x?}")
        }

        /// An expected chunk was missing
        ChunkMissing {
            display("An expected chunk was missing")
        }

        /// Chunk Header was incorrect or invalid in its usage
        ChunkHeaderInvalid(err: [u8; 4]) {
            display("Invalid Chunk header: {err:x?}")
        }

        #[allow(deprecated)]
        #[deprecated]
        /// Some bits were invalid
        ReservedBitSet {
            display("Reserved bits set")
        }

        /// The ALPH chunk preprocessing info flag was invalid
        InvalidAlphaPreprocessing {
            display("Alpha chunk preprocessing flag invalid")
        }

        /// Invalid compression method
        InvalidCompressionMethod {
            display("Invalid compression method")
        }

        /// Alpha chunk doesn't match the frame's size
        AlphaChunkSizeMismatch {
            display("Alpha chunk size mismatch")
        }

        /// Image is too large, either for the platform's pointer size or generally
        ImageTooLarge {
            display("Image too large")
        }

        /// Frame would go out of the canvas
        FrameOutsideImage {
            display("Frame outside image")
        }

        /// Signature of 0x2f not found
        LosslessSignatureInvalid(err: u8) {
            display("Invalid lossless signature: {err:x?}")
        }

        /// Version Number was not zero
        VersionNumberInvalid(err: u8) {
            display("Invalid lossless version number: {err}")
        }

        /// Invalid color cache bits
        InvalidColorCacheBits(err: u8) {
            display("Invalid color cache bits: {err}")
        }

        /// An invalid Huffman code was encountered
        HuffmanError {
            display("Invalid Huffman code")
        }

        /// The bitstream was somehow corrupt
        BitStreamError {
            display("Corrupt bitstream")
        }

        /// The transforms specified were invalid
        TransformError {
            display("Invalid transform")
        }

        /// VP8's `[0x9D, 0x01, 0x2A]` magic not found or invalid
        Vp8MagicInvalid(err: [u8; 3]) {
            display("Invalid VP8 magic: {err:x?}")
        }

        /// VP8 Decoder initialisation wasn't provided with enough data
        NotEnoughInitData {
            display("Not enough VP8 init data")
        }

        /// At time of writing, only the YUV colour-space encoded as `0` is specified
        ColorSpaceInvalid(err: u8) {
            display("Invalid VP8 color space: {err}")
        }

        /// LUMA prediction mode was not recognised
        LumaPredictionModeInvalid(err: i8) {
            display("Invalid VP8 luma prediction mode: {err}")
        }

        /// Intra-prediction mode was not recognised
        IntraPredictionModeInvalid(err: i8) {
            display("Invalid VP8 intra prediction mode: {err}")
        }

        /// Chroma prediction mode was not recognised
        ChromaPredictionModeInvalid(err: i8) {
            display("Invalid VP8 chroma prediction mode: {err}")
        }

        /// Inconsistent image sizes
        InconsistentImageSizes {
            display("Inconsistent image sizes")
        }

        /// The file may be valid, but this crate doesn't support decoding it.
        UnsupportedFeature(err: String) {
            display("Unsupported feature: {err}")
        }

        /// Invalid function call or parameter
        InvalidParameter(err: String) {
            display("Invalid parameter: {err}")
        }

        /// Memory limit exceeded
        MemoryLimitExceeded {
            display("Memory limit exceeded")
        }

        /// Invalid chunk size
        InvalidChunkSize {
            display("Invalid chunk size")
        }

        /// No more frames in image
        NoMoreFrames {
            display("No more frames")
        }
    }
}

/// All possible RIFF chunks in a WebP image file
#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq)]
pub(crate) enum WebPRiffChunk {
    RIFF,
    WEBP,
    VP8,
    VP8L,
    VP8X,
    ANIM,
    ANMF,
    ALPH,
    ICCP,
    EXIF,
    XMP,
    Unknown([u8; 4]),
}

impl WebPRiffChunk {
    pub(crate) const fn from_fourcc(chunk_fourcc: [u8; 4]) -> Self {
        match &chunk_fourcc {
            b"RIFF" => Self::RIFF,
            b"WEBP" => Self::WEBP,
            b"VP8 " => Self::VP8,
            b"VP8L" => Self::VP8L,
            b"VP8X" => Self::VP8X,
            b"ANIM" => Self::ANIM,
            b"ANMF" => Self::ANMF,
            b"ALPH" => Self::ALPH,
            b"ICCP" => Self::ICCP,
            b"EXIF" => Self::EXIF,
            b"XMP " => Self::XMP,
            _ => Self::Unknown(chunk_fourcc),
        }
    }

    pub(crate) const fn to_fourcc(self) -> [u8; 4] {
        match self {
            Self::RIFF => *b"RIFF",
            Self::WEBP => *b"WEBP",
            Self::VP8 => *b"VP8 ",
            Self::VP8L => *b"VP8L",
            Self::VP8X => *b"VP8X",
            Self::ANIM => *b"ANIM",
            Self::ANMF => *b"ANMF",
            Self::ALPH => *b"ALPH",
            Self::ICCP => *b"ICCP",
            Self::EXIF => *b"EXIF",
            Self::XMP => *b"XMP ",
            Self::Unknown(fourcc) => fourcc,
        }
    }

    pub(crate) const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

// enum WebPImage {
//     Lossy(VP8Frame),
//     Lossless(LosslessFrame),
//     Extended(ExtendedImage),
// }

enum ImageKind {
    Lossy,
    Lossless,
    Extended(WebPExtendedInfo),
}

struct AnimationState {
    next_frame: u32,
    next_frame_start: u64,
    dispose_next_frame: bool,
    previous_frame_width: u32,
    previous_frame_height: u32,
    previous_frame_x_offset: u32,
    previous_frame_y_offset: u32,
    canvas: Option<Vec<u8>>,
}
impl Default for AnimationState {
    fn default() -> Self {
        Self {
            next_frame: 0,
            next_frame_start: 0,
            dispose_next_frame: true,
            previous_frame_width: 0,
            previous_frame_height: 0,
            previous_frame_x_offset: 0,
            previous_frame_y_offset: 0,
            canvas: None,
        }
    }
}

/// Number of times that an animation loops.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LoopCount {
    /// The animation loops forever.
    Forever,
    /// Each frame of the animation is displayed the specified number of times.
    Times(NonZeroU16),
}

/// WebP decoder configuration options
#[derive(Clone)]
#[non_exhaustive]
pub struct WebPDecodeOptions {
    /// The upsampling method used in conversion from lossy yuv to rgb
    ///
    /// Defaults to `Bilinear`.
    pub lossy_upsampling: UpsamplingMethod,
}

impl Default for WebPDecodeOptions {
    fn default() -> Self {
        Self {
            lossy_upsampling: UpsamplingMethod::Bilinear,
        }
    }
}

/// Methods for upsampling the chroma values in lossy decoding
///
/// The chroma red and blue planes are encoded in VP8 as half the size of the luma plane
/// Therefore we need to upsample these values up to fit each pixel in the image.
#[derive(Clone, Copy, Default)]
pub enum UpsamplingMethod {
    /// Fancy upsampling
    ///
    /// Does bilinear interpolation using the 4 values nearest to the pixel, weighting based on the distance
    /// from the pixel.
    #[default]
    Bilinear,
    /// Simple upsampling, just uses the closest u/v value to the pixel when upsampling
    ///
    /// Matches the -nofancy option in dwebp.
    /// Should be faster but may lead to slightly jagged edges.
    Simple,
}

/// photo-gallery: what a decoder allocates as it reads an image (the chunk map, the VP8 and lossless decoders'
/// buffers and tables, the alpha plane, an animation's frame and canvas), kept from one image to the next:
/// [`WebPDecoder::new_with_scratch`], [`WebPDecoder::into_scratch`]. A decoder reads in them exactly as in new ones.
#[derive(Default)]
pub struct Scratch {
    chunks: HashMap<WebPRiffChunk, Range<u64>>,
    buffers: Buffers,
}

impl Scratch {
    /// Free the buffers that hold more than `bytes`, after an image far larger than the rest.
    pub fn trim(&mut self, bytes: usize) {
        let Buffers {
            vp8,
            lossless,
            argb,
            alpha,
            frame,
            canvas,
        } = &mut self.buffers;
        vp8.trim(bytes);
        lossless.trim(bytes);
        for buf in [argb, alpha, frame, canvas] {
            if buf.capacity() > bytes {
                *buf = Vec::new();
            }
        }
    }
}

/// photo-gallery: a decoder's kept buffers.
#[derive(Default)]
pub(crate) struct Buffers {
    pub(crate) vp8: Vp8Scratch,
    pub(crate) lossless: LosslessScratch,
    /// A lossless image's RGBA when the output is RGB, or an alpha plane's.
    pub(crate) argb: Vec<u8>,
    /// The alpha plane.
    pub(crate) alpha: Vec<u8>,
    /// An animation's frame.
    frame: Vec<u8>,
    /// An animation's canvas.
    canvas: Vec<u8>,
}

/// photo-gallery: `buf`, taken from where it is kept, as `vec![0; len]` would be.
pub(crate) fn zeroed(buf: &mut Vec<u8>, len: usize) -> Vec<u8> {
    let mut v = std::mem::take(buf);
    v.clear();
    v.resize(len, 0);
    v
}

/// WebP image format decoder.
pub struct WebPDecoder<R> {
    r: R,
    memory_limit: usize,

    width: u32,
    height: u32,

    kind: ImageKind,
    animation: AnimationState,

    is_lossy: bool,
    has_alpha: bool,
    num_frames: u32,
    loop_count: LoopCount,
    loop_duration: u64,

    chunks: HashMap<WebPRiffChunk, Range<u64>>,

    webp_decode_options: WebPDecodeOptions,

    /// photo-gallery: the buffers to decode in.
    buffers: Buffers,
}

impl<R: BufRead + Seek> WebPDecoder<R> {
    /// Create a new `WebPDecoder` from the reader `r`. The decoder performs many small reads, so the
    /// reader should be buffered.
    pub fn new(r: R) -> Result<Self, DecodingError> {
        Self::new_with_options(r, WebPDecodeOptions::default())
    }

    /// Create a new `WebPDecoder` from the reader `r` with the options `WebPDecodeOptions`. The decoder
    /// performs many small reads, so the reader should be buffered.
    pub fn new_with_options(
        r: R,
        webp_decode_options: WebPDecodeOptions,
    ) -> Result<Self, DecodingError> {
        Self::new_with_options_and_scratch(r, webp_decode_options, Scratch::default())
            .map_err(|(err, _)| err)
    }

    /// photo-gallery: [`Self::new`] in the buffers of an earlier decoder ([`Self::into_scratch`]), which come
    /// back with the error if the headers can't be read.
    pub fn new_with_scratch(r: R, scratch: Scratch) -> Result<Self, (DecodingError, Scratch)> {
        Self::new_with_options_and_scratch(r, WebPDecodeOptions::default(), scratch)
    }

    /// photo-gallery: [`Self::new_with_options`] in kept buffers, as [`Self::new_with_scratch`].
    pub fn new_with_options_and_scratch(
        r: R,
        webp_decode_options: WebPDecodeOptions,
        scratch: Scratch,
    ) -> Result<Self, (DecodingError, Scratch)> {
        let Scratch {
            mut chunks,
            buffers,
        } = scratch;
        chunks.clear();
        let mut decoder = Self {
            r,
            width: 0,
            height: 0,
            num_frames: 0,
            kind: ImageKind::Lossy,
            chunks,
            animation: Default::default(),
            memory_limit: usize::MAX,
            is_lossy: false,
            has_alpha: false,
            loop_count: LoopCount::Times(NonZeroU16::new(1).unwrap()),
            loop_duration: 0,
            webp_decode_options,
            buffers,
        };
        match decoder.read_data() {
            Ok(()) => Ok(decoder),
            Err(err) => Err((err, decoder.into_scratch())),
        }
    }

    /// photo-gallery: the buffers, for [`Self::new_with_scratch`].
    pub fn into_scratch(self) -> Scratch {
        let mut buffers = self.buffers;
        if let Some(canvas) = self.animation.canvas {
            buffers.canvas = canvas;
        }
        Scratch {
            chunks: self.chunks,
            buffers,
        }
    }

    fn read_data(&mut self) -> Result<(), DecodingError> {
        let (WebPRiffChunk::RIFF, riff_size, _) = read_chunk_header(&mut self.r)? else {
            return Err(DecodingError::ChunkHeaderInvalid(*b"RIFF"));
        };

        match &read_fourcc(&mut self.r)? {
            WebPRiffChunk::WEBP => {}
            fourcc => return Err(DecodingError::WebpSignatureInvalid(fourcc.to_fourcc())),
        }

        let (chunk, chunk_size, chunk_size_rounded) = read_chunk_header(&mut self.r)?;
        let start = self.r.stream_position()?;

        match chunk {
            WebPRiffChunk::VP8 => {
                let tag = self.r.read_u24::<LittleEndian>()?;

                let keyframe = tag & 1 == 0;
                if !keyframe {
                    return Err(DecodingError::UnsupportedFeature(
                        "Non-keyframe frames".to_owned(),
                    ));
                }

                let mut tag = [0u8; 3];
                self.r.read_exact(&mut tag)?;
                if tag != [0x9d, 0x01, 0x2a] {
                    return Err(DecodingError::Vp8MagicInvalid(tag));
                }

                let w = self.r.read_u16::<LittleEndian>()?;
                let h = self.r.read_u16::<LittleEndian>()?;

                self.width = u32::from(w & 0x3FFF);
                self.height = u32::from(h & 0x3FFF);
                if self.width == 0 || self.height == 0 {
                    return Err(DecodingError::InconsistentImageSizes);
                }

                self.chunks
                    .insert(WebPRiffChunk::VP8, start..start + chunk_size);
                self.kind = ImageKind::Lossy;
                self.is_lossy = true;
            }
            WebPRiffChunk::VP8L => {
                let signature = self.r.read_u8()?;
                if signature != 0x2f {
                    return Err(DecodingError::LosslessSignatureInvalid(signature));
                }

                let header = self.r.read_u32::<LittleEndian>()?;
                let version = header >> 29;
                if version != 0 {
                    return Err(DecodingError::VersionNumberInvalid(version as u8));
                }

                self.width = (1 + header) & 0x3FFF;
                self.height = (1 + (header >> 14)) & 0x3FFF;
                self.chunks
                    .insert(WebPRiffChunk::VP8L, start..start + chunk_size);
                self.kind = ImageKind::Lossless;
                self.has_alpha = (header >> 28) & 1 != 0;
            }
            WebPRiffChunk::VP8X => {
                let mut info = extended::read_extended_header(&mut self.r)?;
                self.width = info.canvas_width;
                self.height = info.canvas_height;

                let mut position = start + chunk_size_rounded;
                let max_position = position + riff_size.saturating_sub(12);
                self.r.seek(io::SeekFrom::Start(position))?;

                while position < max_position {
                    match read_chunk_header(&mut self.r) {
                        Ok((chunk, chunk_size, chunk_size_rounded)) => {
                            let range = position + 8..position + 8 + chunk_size;
                            position += 8 + chunk_size_rounded;

                            if !chunk.is_unknown() {
                                self.chunks.entry(chunk).or_insert(range);
                            }

                            if chunk == WebPRiffChunk::ANMF {
                                self.num_frames += 1;
                                if chunk_size < 24 {
                                    return Err(DecodingError::InvalidChunkSize);
                                }

                                self.r.seek_relative(12)?;
                                let duration = self.r.read_u32::<LittleEndian>()? & 0xffffff;
                                self.loop_duration =
                                    self.loop_duration.wrapping_add(u64::from(duration));

                                // If the image is animated, the image data chunk will be inside the
                                // ANMF chunks, so we must inspect them to determine whether the
                                // image contains any lossy image data. VP8 chunks store lossy data
                                // and the spec says that lossless images SHOULD NOT contain ALPH
                                // chunks, so we treat both as indicators of lossy images.
                                if !self.is_lossy {
                                    let (subchunk, ..) = read_chunk_header(&mut self.r)?;
                                    if let WebPRiffChunk::VP8 | WebPRiffChunk::ALPH = subchunk {
                                        self.is_lossy = true;
                                    }
                                    self.r.seek_relative(chunk_size_rounded as i64 - 24)?;
                                } else {
                                    self.r.seek_relative(chunk_size_rounded as i64 - 16)?;
                                }

                                continue;
                            }

                            self.r.seek_relative(chunk_size_rounded as i64)?;
                        }
                        Err(DecodingError::IoError(e))
                            if e.kind() == io::ErrorKind::UnexpectedEof =>
                        {
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                }
                self.is_lossy = self.is_lossy || self.chunks.contains_key(&WebPRiffChunk::VP8);

                // NOTE: We allow malformed images that have `info.icc_profile` set without a ICCP chunk,
                // because this is relatively common.
                if info.animation
                    && (!self.chunks.contains_key(&WebPRiffChunk::ANIM)
                        || !self.chunks.contains_key(&WebPRiffChunk::ANMF))
                    || info.exif_metadata && !self.chunks.contains_key(&WebPRiffChunk::EXIF)
                    || info.xmp_metadata && !self.chunks.contains_key(&WebPRiffChunk::XMP)
                    || !info.animation
                        && self.chunks.contains_key(&WebPRiffChunk::VP8)
                            == self.chunks.contains_key(&WebPRiffChunk::VP8L)
                {
                    return Err(DecodingError::ChunkMissing);
                }

                // Decode ANIM chunk.
                if info.animation {
                    match self.read_chunk(WebPRiffChunk::ANIM, 6) {
                        Ok(Some(chunk)) => {
                            let mut cursor = Cursor::new(chunk);
                            cursor.read_exact(&mut info.background_color_hint)?;
                            self.loop_count = match cursor.read_u16::<LittleEndian>()? {
                                0 => LoopCount::Forever,
                                n => LoopCount::Times(NonZeroU16::new(n).unwrap()),
                            };
                            self.animation.next_frame_start =
                                self.chunks.get(&WebPRiffChunk::ANMF).unwrap().start - 8;
                        }
                        Ok(None) => return Err(DecodingError::ChunkMissing),
                        Err(DecodingError::MemoryLimitExceeded) => {
                            return Err(DecodingError::InvalidChunkSize)
                        }
                        Err(e) => return Err(e),
                    }
                }

                // If the image is animated, the image data chunk will be inside the ANMF chunks. We
                // store the ALPH, VP8, and VP8L chunks (as applicable) of the first frame in the
                // hashmap so that we can read them later.
                if let Some(range) = self.chunks.get(&WebPRiffChunk::ANMF).cloned() {
                    let mut position = range.start + 16;
                    self.r.seek(io::SeekFrom::Start(position))?;
                    for _ in 0..2 {
                        let (subchunk, subchunk_size, subchunk_size_rounded) =
                            read_chunk_header(&mut self.r)?;
                        let subrange = position + 8..position + 8 + subchunk_size;
                        self.chunks.entry(subchunk).or_insert(subrange.clone());

                        position += 8 + subchunk_size_rounded;
                        if position + 8 > range.end {
                            break;
                        }
                    }
                }

                self.has_alpha = info.alpha;
                self.kind = ImageKind::Extended(info);
            }
            _ => return Err(DecodingError::ChunkHeaderInvalid(chunk.to_fourcc())),
        };

        Ok(())
    }

    /// Sets the maximum amount of memory that the decoder is allowed to allocate at once.
    ///
    /// TODO: Some allocations currently ignore this limit.
    pub fn set_memory_limit(&mut self, limit: usize) {
        self.memory_limit = limit;
    }

    /// Get the background color specified in the image file if the image is extended and animated webp.
    pub fn background_color_hint(&self) -> Option<[u8; 4]> {
        if let ImageKind::Extended(info) = &self.kind {
            Some(info.background_color_hint)
        } else {
            None
        }
    }

    /// Sets the background color if the image is an extended and animated webp.
    pub fn set_background_color(&mut self, color: [u8; 4]) -> Result<(), DecodingError> {
        if let ImageKind::Extended(info) = &mut self.kind {
            info.background_color = Some(color);
            Ok(())
        } else {
            Err(DecodingError::InvalidParameter(
                "Background color can only be set on animated webp".to_owned(),
            ))
        }
    }

    /// Returns the (width, height) of the image in pixels.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns whether the image has an alpha channel. If so, the pixel format is Rgba8 and
    /// otherwise Rgb8.
    pub fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    /// Returns true if the image is animated.
    pub fn is_animated(&self) -> bool {
        match &self.kind {
            ImageKind::Lossy | ImageKind::Lossless => false,
            ImageKind::Extended(extended) => extended.animation,
        }
    }

    /// Returns whether the image is lossy. For animated images, this is true if any frame is lossy.
    pub fn is_lossy(&mut self) -> bool {
        self.is_lossy
    }

    /// Returns the number of frames of a single loop of the animation, or zero if the image is not
    /// animated.
    pub fn num_frames(&self) -> u32 {
        self.num_frames
    }

    /// Returns the number of times the animation should loop.
    pub fn loop_count(&self) -> LoopCount {
        self.loop_count
    }

    /// Returns the total duration of one loop through the animation in milliseconds, or zero if the
    /// image is not animated.
    ///
    /// This is the sum of the durations of all individual frames of the image.
    pub fn loop_duration(&self) -> u64 {
        self.loop_duration
    }

    fn read_chunk(
        &mut self,
        chunk: WebPRiffChunk,
        max_size: usize,
    ) -> Result<Option<Vec<u8>>, DecodingError> {
        match self.chunks.get(&chunk) {
            Some(range) => {
                if range.end - range.start > max_size as u64 {
                    return Err(DecodingError::MemoryLimitExceeded);
                }

                self.r.seek(io::SeekFrom::Start(range.start))?;
                let mut data = vec![0; (range.end - range.start) as usize];
                self.r.read_exact(&mut data)?;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    /// Returns the raw bytes of the ICC profile, or None if there is no ICC profile.
    pub fn icc_profile(&mut self) -> Result<Option<Vec<u8>>, DecodingError> {
        self.read_chunk(WebPRiffChunk::ICCP, self.memory_limit)
    }

    /// Returns the raw bytes of the EXIF metadata, or None if there is no EXIF metadata.
    pub fn exif_metadata(&mut self) -> Result<Option<Vec<u8>>, DecodingError> {
        self.read_chunk(WebPRiffChunk::EXIF, self.memory_limit)
    }

    /// Returns the raw bytes of the XMP metadata, or None if there is no XMP metadata.
    pub fn xmp_metadata(&mut self) -> Result<Option<Vec<u8>>, DecodingError> {
        self.read_chunk(WebPRiffChunk::XMP, self.memory_limit)
    }

    /// Returns the number of bytes required to store the image or a single frame, or None if that
    /// would take more than `usize::MAX` bytes.
    pub fn output_buffer_size(&self) -> Option<usize> {
        let bytes_per_pixel = if self.has_alpha() { 4 } else { 3 };
        (self.width as usize)
            .checked_mul(self.height as usize)?
            .checked_mul(bytes_per_pixel)
    }

    /// Returns the raw bytes of the image. For animated images, this is the first frame.
    ///
    /// Fails with `ImageTooLarge` if `buf` has length different than `output_buffer_size()`
    pub fn read_image(&mut self, buf: &mut [u8]) -> Result<(), DecodingError> {
        if Some(buf.len()) != self.output_buffer_size() {
            return Err(DecodingError::ImageTooLarge);
        }

        if self.is_animated() {
            let saved = std::mem::take(&mut self.animation);
            self.animation.next_frame_start =
                self.chunks.get(&WebPRiffChunk::ANMF).unwrap().start - 8;
            let result = self.read_frame(buf);
            // photo-gallery: the canvas kept.
            if let Some(canvas) = self.animation.canvas.take() {
                self.buffers.canvas = canvas;
            }
            self.animation = saved;
            result?;
        } else if let Some(range) = self.chunks.get(&WebPRiffChunk::VP8L) {
            // photo-gallery: in the kept buffers, given back whatever happens.
            let mut decoder = LosslessDecoder::with_scratch(
                range_reader(&mut self.r, range.clone())?,
                std::mem::take(&mut self.buffers.lossless),
            );

            let result = if self.has_alpha {
                decoder.decode_frame(self.width, self.height, false, buf)
            } else {
                let len = self.width as usize * self.height as usize * 4;
                let mut data = zeroed(&mut self.buffers.argb, len);
                let result = decoder.decode_frame(self.width, self.height, false, &mut data);
                if result.is_ok() {
                    for (rgba_val, chunk) in data.chunks_exact(4).zip(buf.chunks_exact_mut(3)) {
                        chunk.copy_from_slice(&rgba_val[..3]);
                    }
                }
                self.buffers.argb = data;
                result
            };
            self.buffers.lossless = decoder.into_scratch();
            result?;
        } else {
            let range = self
                .chunks
                .get(&WebPRiffChunk::VP8)
                .ok_or(DecodingError::ChunkMissing)?;
            let reader = range_reader(&mut self.r, range.start..range.end)?;
            // photo-gallery: in the kept buffers; the frame's planes go back once read.
            let frame = Vp8Decoder::decode_frame_with(reader, &mut self.buffers.vp8)?;
            let result = self.read_lossy(frame, buf);
            result?;
        }

        Ok(())
    }

    /// photo-gallery: the rest of `read_image` for a lossy image (upstream's, moved here so that the frame's
    /// planes go back to the kept buffers whatever happens).
    fn read_lossy(
        &mut self,
        frame: super::vp8::Frame,
        buf: &mut [u8],
    ) -> Result<(), DecodingError> {
        let result = self.read_lossy_frame(&frame, buf);
        self.buffers.vp8.recycle(frame);
        result
    }

    fn read_lossy_frame(
        &mut self,
        frame: &super::vp8::Frame,
        buf: &mut [u8],
    ) -> Result<(), DecodingError> {
        {
            if u32::from(frame.width) != self.width || u32::from(frame.height) != self.height {
                return Err(DecodingError::InconsistentImageSizes);
            }

            if self.has_alpha() {
                frame.fill_rgba(buf, self.webp_decode_options.lossy_upsampling);

                let range = self
                    .chunks
                    .get(&WebPRiffChunk::ALPH)
                    .ok_or(DecodingError::ChunkMissing)?
                    .clone();
                let alpha_chunk = read_alpha_chunk(
                    &mut range_reader(&mut self.r, range)?,
                    self.width as u16,
                    self.height as u16,
                    &mut self.buffers,
                )?;

                for y in 0..frame.height {
                    for x in 0..frame.width {
                        let predictor: u8 = get_alpha_predictor(
                            x.into(),
                            y.into(),
                            frame.width.into(),
                            alpha_chunk.filtering_method,
                            buf,
                        );

                        let alpha_index =
                            usize::from(y) * usize::from(frame.width) + usize::from(x);
                        let buffer_index = alpha_index * 4 + 3;

                        buf[buffer_index] = predictor.wrapping_add(alpha_chunk.data[alpha_index]);
                    }
                }
                // photo-gallery: the alpha plane kept.
                self.buffers.alpha = alpha_chunk.data;
            } else {
                frame.fill_rgb(buf, self.webp_decode_options.lossy_upsampling);
            }
        }

        Ok(())
    }

    /// Reads the next frame of the animation.
    ///
    /// The frame contents are written into `buf` and the method returns the duration of the frame
    /// in milliseconds. If there are no more frames, the method returns
    /// `DecodingError::NoMoreFrames` and `buf` is left unchanged.
    ///
    /// # Panics
    ///
    /// Panics if the image is not animated.
    pub fn read_frame(&mut self, buf: &mut [u8]) -> Result<u32, DecodingError> {
        assert!(self.is_animated());
        assert_eq!(Some(buf.len()), self.output_buffer_size());

        if self.animation.next_frame == self.num_frames {
            return Err(DecodingError::NoMoreFrames);
        }

        let ImageKind::Extended(info) = &self.kind else {
            unreachable!()
        };

        self.r
            .seek(io::SeekFrom::Start(self.animation.next_frame_start))?;

        let anmf_size = match read_chunk_header(&mut self.r)? {
            (WebPRiffChunk::ANMF, size, _) if size >= 32 => size,
            _ => return Err(DecodingError::ChunkHeaderInvalid(*b"ANMF")),
        };

        // Read ANMF chunk
        let frame_x = extended::read_3_bytes(&mut self.r)? * 2;
        let frame_y = extended::read_3_bytes(&mut self.r)? * 2;
        let frame_width = extended::read_3_bytes(&mut self.r)? + 1;
        let frame_height = extended::read_3_bytes(&mut self.r)? + 1;
        if frame_width > 16384 || frame_height > 16384 {
            return Err(DecodingError::ImageTooLarge);
        }
        if frame_x + frame_width > self.width || frame_y + frame_height > self.height {
            return Err(DecodingError::FrameOutsideImage);
        }
        let duration = extended::read_3_bytes(&mut self.r)?;
        let frame_info = self.r.read_u8()?;
        let use_alpha_blending = frame_info & 0b00000010 == 0;
        let dispose = frame_info & 0b00000001 != 0;

        let clear_color = if self.animation.dispose_next_frame {
            info.background_color
        } else {
            None
        };

        // Read normal bitstream now
        let (chunk, chunk_size, chunk_size_rounded) = read_chunk_header(&mut self.r)?;
        if chunk_size_rounded + 24 > anmf_size {
            return Err(DecodingError::ChunkHeaderInvalid(chunk.to_fourcc()));
        }

        // photo-gallery: the frame, the VP8 and lossless decoders and the alpha plane in the kept buffers.
        let (frame, frame_has_alpha): (Vec<u8>, bool) = match chunk {
            WebPRiffChunk::VP8 => {
                let reader = (&mut self.r).take(chunk_size);
                let raw_frame = Vp8Decoder::decode_frame_with(reader, &mut self.buffers.vp8)?;
                if u32::from(raw_frame.width) != frame_width
                    || u32::from(raw_frame.height) != frame_height
                {
                    self.buffers.vp8.recycle(raw_frame);
                    return Err(DecodingError::InconsistentImageSizes);
                }
                let len = frame_width as usize * frame_height as usize * 3;
                let mut rgb_frame = zeroed(&mut self.buffers.frame, len);
                raw_frame.fill_rgb(&mut rgb_frame, self.webp_decode_options.lossy_upsampling);
                self.buffers.vp8.recycle(raw_frame);
                (rgb_frame, false)
            }
            WebPRiffChunk::VP8L => {
                let reader = (&mut self.r).take(chunk_size);
                let mut lossless_decoder = LosslessDecoder::with_scratch(
                    reader,
                    std::mem::take(&mut self.buffers.lossless),
                );
                let len = frame_width as usize * frame_height as usize * 4;
                let mut rgba_frame = zeroed(&mut self.buffers.frame, len);
                let result = lossless_decoder.decode_frame(
                    frame_width,
                    frame_height,
                    false,
                    &mut rgba_frame,
                );
                self.buffers.lossless = lossless_decoder.into_scratch();
                if let Err(err) = result {
                    self.buffers.frame = rgba_frame;
                    return Err(err);
                }
                (rgba_frame, true)
            }
            WebPRiffChunk::ALPH => {
                if chunk_size_rounded + 32 > anmf_size {
                    return Err(DecodingError::ChunkHeaderInvalid(chunk.to_fourcc()));
                }

                // read alpha
                let next_chunk_start = self.r.stream_position()? + chunk_size_rounded;
                let mut reader = (&mut self.r).take(chunk_size);
                let alpha_chunk = read_alpha_chunk(
                    &mut reader,
                    frame_width as u16,
                    frame_height as u16,
                    &mut self.buffers,
                )?;

                // read opaque
                // photo-gallery: the alpha plane given back to the kept buffers if this fails.
                let opaque = (|| {
                    self.r.seek(io::SeekFrom::Start(next_chunk_start))?;
                    let (next_chunk, next_chunk_size, _) = read_chunk_header(&mut self.r)?;
                    if chunk_size + next_chunk_size + 32 > anmf_size {
                        return Err(DecodingError::ChunkHeaderInvalid(next_chunk.to_fourcc()));
                    }

                    Vp8Decoder::decode_frame_with(
                        (&mut self.r).take(next_chunk_size),
                        &mut self.buffers.vp8,
                    )
                })();
                let frame = match opaque {
                    Ok(frame) => frame,
                    Err(err) => {
                        self.buffers.alpha = alpha_chunk.data;
                        return Err(err);
                    }
                };

                let len = frame_width as usize * frame_height as usize * 4;
                let mut rgba_frame = zeroed(&mut self.buffers.frame, len);
                frame.fill_rgba(&mut rgba_frame, self.webp_decode_options.lossy_upsampling);

                for y in 0..frame.height {
                    for x in 0..frame.width {
                        let predictor: u8 = get_alpha_predictor(
                            x.into(),
                            y.into(),
                            frame.width.into(),
                            alpha_chunk.filtering_method,
                            &rgba_frame,
                        );

                        let alpha_index =
                            usize::from(y) * usize::from(frame.width) + usize::from(x);
                        let buffer_index = alpha_index * 4 + 3;

                        rgba_frame[buffer_index] =
                            predictor.wrapping_add(alpha_chunk.data[alpha_index]);
                    }
                }
                self.buffers.vp8.recycle(frame);
                self.buffers.alpha = alpha_chunk.data;

                (rgba_frame, true)
            }
            _ => return Err(DecodingError::ChunkHeaderInvalid(chunk.to_fourcc())),
        };

        // fill starting canvas with clear color
        if self.animation.canvas.is_none() {
            self.animation.canvas = {
                // photo-gallery: in the kept buffer.
                let mut canvas = zeroed(
                    &mut self.buffers.canvas,
                    (self.width * self.height * 4) as usize,
                );
                if let Some(color) = info.background_color.as_ref() {
                    canvas
                        .chunks_exact_mut(4)
                        .for_each(|c| c.copy_from_slice(color))
                }
                Some(canvas)
            }
        }
        extended::composite_frame(
            self.animation.canvas.as_mut().unwrap(),
            self.width,
            self.height,
            clear_color,
            &frame,
            frame_x,
            frame_y,
            frame_width,
            frame_height,
            frame_has_alpha,
            use_alpha_blending,
            self.animation.previous_frame_width,
            self.animation.previous_frame_height,
            self.animation.previous_frame_x_offset,
            self.animation.previous_frame_y_offset,
        );
        // photo-gallery: the frame kept.
        self.buffers.frame = frame;

        self.animation.previous_frame_width = frame_width;
        self.animation.previous_frame_height = frame_height;
        self.animation.previous_frame_x_offset = frame_x;
        self.animation.previous_frame_y_offset = frame_y;

        self.animation.dispose_next_frame = dispose;
        self.animation.next_frame_start += anmf_size + 8;
        self.animation.next_frame += 1;

        if self.has_alpha() {
            buf.copy_from_slice(self.animation.canvas.as_ref().unwrap());
        } else {
            for (b, c) in buf
                .chunks_exact_mut(3)
                .zip(self.animation.canvas.as_ref().unwrap().chunks_exact(4))
            {
                b.copy_from_slice(&c[..3]);
            }
        }

        Ok(duration)
    }

    /// Resets the animation to the first frame.
    ///
    /// # Panics
    ///
    /// Panics if the image is not animated.
    pub fn reset_animation(&mut self) {
        assert!(self.is_animated());

        self.animation.next_frame = 0;
        self.animation.next_frame_start = self.chunks.get(&WebPRiffChunk::ANMF).unwrap().start - 8;
        self.animation.dispose_next_frame = true;
    }

    /// Sets the upsampling method that is used in lossy decoding
    pub fn set_lossy_upsampling(&mut self, upsampling_method: UpsamplingMethod) {
        self.webp_decode_options.lossy_upsampling = upsampling_method;
    }
}

pub(crate) fn range_reader<R: BufRead + Seek>(
    mut r: R,
    range: Range<u64>,
) -> Result<impl BufRead, DecodingError> {
    r.seek(io::SeekFrom::Start(range.start))?;
    Ok(r.take(range.end - range.start))
}

pub(crate) fn read_fourcc<R: BufRead>(mut r: R) -> Result<WebPRiffChunk, DecodingError> {
    let mut chunk_fourcc = [0; 4];
    r.read_exact(&mut chunk_fourcc)?;
    Ok(WebPRiffChunk::from_fourcc(chunk_fourcc))
}

pub(crate) fn read_chunk_header<R: BufRead>(
    mut r: R,
) -> Result<(WebPRiffChunk, u64, u64), DecodingError> {
    let chunk = read_fourcc(&mut r)?;
    let chunk_size = r.read_u32::<LittleEndian>()?;
    let chunk_size_rounded = chunk_size.saturating_add(chunk_size & 1);
    Ok((chunk, chunk_size.into(), chunk_size_rounded.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    const RGB_BPP: usize = 3;

    #[test]
    fn add_with_overflow_size() {
        let bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0xaf, 0x37, 0x80, 0x47, 0x57, 0x45, 0x42, 0x50, 0x6c, 0x64,
            0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xfb, 0x7e, 0x73, 0x00, 0x06, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65,
            0x40, 0xfb, 0xff, 0xff, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65,
            0x00, 0x00, 0x00, 0x00, 0x62, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49,
            0x49, 0x54, 0x55, 0x50, 0x4c, 0x54, 0x59, 0x50, 0x45, 0x33, 0x37, 0x44, 0x4d, 0x46,
        ];

        let data = std::io::Cursor::new(bytes);

        let _ = WebPDecoder::new(data);
    }

    #[test]
    fn decode_2x2_single_color_image() {
        // Image data created from imagemagick and output of xxd:
        // $ convert -size 2x2 xc:#f00 red.webp
        // $ xxd -g 1 red.webp | head

        const NUM_PIXELS: usize = 2 * 2 * RGB_BPP;
        // 2x2 red pixel image
        let bytes = [
            0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x20, 0x30, 0x00, 0x00, 0x00, 0xd0, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x02, 0x00,
            0x02, 0x00, 0x02, 0x00, 0x34, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x01, 0xf8, 0x00, 0x03,
            0xb0, 0x00, 0xfe, 0xf0, 0xc4, 0x0b, 0xff, 0x20, 0xb9, 0x61, 0x75, 0xc8, 0xd7, 0xff,
            0x20, 0x3f, 0xe4, 0x07, 0xfc, 0x80, 0xff, 0xf8, 0xf2, 0x00, 0x00, 0x00,
        ];

        let mut data = [0; NUM_PIXELS];
        let mut decoder = WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
        decoder.read_image(&mut data).unwrap();

        // All pixels are the same value
        let first_pixel = &data[..RGB_BPP];
        assert!(data.chunks_exact(3).all(|ch| ch.iter().eq(first_pixel)));
    }

    #[test]
    fn decode_3x3_single_color_image() {
        // Test that any odd pixel "tail" is decoded properly

        const NUM_PIXELS: usize = 3 * 3 * RGB_BPP;
        // 3x3 red pixel image
        let bytes = [
            0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x20, 0x30, 0x00, 0x00, 0x00, 0xd0, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x03, 0x00,
            0x03, 0x00, 0x02, 0x00, 0x34, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x01, 0xf8, 0x00, 0x03,
            0xb0, 0x00, 0xfe, 0xf0, 0xc4, 0x0b, 0xff, 0x20, 0xb9, 0x61, 0x75, 0xc8, 0xd7, 0xff,
            0x20, 0x3f, 0xe4, 0x07, 0xfc, 0x80, 0xff, 0xf8, 0xf2, 0x00, 0x00, 0x00,
        ];

        let mut data = [0; NUM_PIXELS];
        let mut decoder = WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
        decoder.read_image(&mut data).unwrap();

        // All pixels are the same value
        let first_pixel = &data[..RGB_BPP];
        assert!(data.chunks_exact(3).all(|ch| ch.iter().eq(first_pixel)));
    }
}

/// photo-gallery: a decoder in kept buffers ([`Scratch`]) decodes as a new one, whatever it decoded before.
#[cfg(test)]
mod photo_gallery {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// What a decoder gives for a file: its headers, the image, and (animations) each frame, then the error after
    /// the last; or the panic.
    #[derive(Debug, PartialEq)]
    enum Decoded {
        Headers(String),
        Read {
            dimensions: (u32, u32),
            alpha: bool,
            frames: u32,
            image: Result<Vec<u8>, String>,
            animation: Vec<Result<(u32, Vec<u8>), String>>,
        },
        TooLarge,
        Panicked,
    }

    fn decode(bytes: &[u8], scratch: Scratch, background: bool) -> (Decoded, Scratch) {
        let mut decoder = match WebPDecoder::new_with_scratch(Cursor::new(bytes), scratch) {
            Ok(decoder) => decoder,
            Err((err, scratch)) => return (Decoded::Headers(format!("{err:?}")), scratch),
        };
        let Some(len) = decoder.output_buffer_size().filter(|&len| len <= 1 << 24) else {
            return (Decoded::TooLarge, decoder.into_scratch());
        };
        let mut image = vec![0; len];
        let image = match decoder.read_image(&mut image) {
            Ok(()) => Ok(image),
            Err(err) => Err(format!("{err:?}")),
        };
        let mut animation = Vec::new();
        if decoder.is_animated() {
            if background {
                decoder.set_background_color([10, 20, 30, 40]).unwrap();
            }
            for _ in 0..=decoder.num_frames() {
                let mut frame = vec![0; len];
                match decoder.read_frame(&mut frame) {
                    Ok(duration) => animation.push(Ok((duration, frame))),
                    Err(err) => {
                        animation.push(Err(format!("{err:?}")));
                        break;
                    }
                }
            }
        }
        let decoded = Decoded::Read {
            dimensions: decoder.dimensions(),
            alpha: decoder.has_alpha(),
            frames: decoder.num_frames(),
            image,
            animation,
        };
        (decoded, decoder.into_scratch())
    }

    /// `decode`, its panics caught: the scratch, gone with the decoder, is then a new one.
    fn decode_caught(bytes: &[u8], scratch: Scratch, background: bool) -> (Decoded, Scratch) {
        catch_unwind(AssertUnwindSafe(|| decode(bytes, scratch, background)))
            .unwrap_or_else(|_| (Decoded::Panicked, Scratch::default()))
    }

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut chunk = tag.to_vec();
        chunk.extend((body.len() as u32).to_le_bytes());
        chunk.extend(body);
        if body.len() % 2 == 1 {
            chunk.push(0);
        }
        chunk
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut file = b"RIFF".to_vec();
        file.extend((body.len() as u32 + 4).to_le_bytes());
        file.extend(b"WEBP");
        file.extend(body);
        file
    }

    /// A file's chunks, whole (header and padding included).
    fn chunks_of(file: &[u8]) -> Vec<Vec<u8>> {
        let mut chunks = Vec::new();
        let mut at = 12;
        while at + 8 <= file.len() {
            let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            let end = (at + 8 + size + size % 2).min(file.len());
            chunks.push(file[at..end].to_vec());
            at = end;
        }
        chunks
    }

    fn u24(value: u32) -> [u8; 3] {
        let [a, b, c, _] = value.to_le_bytes();
        [a, b, c]
    }

    fn vp8x(flags: u8, (width, height): (u32, u32)) -> Vec<u8> {
        let mut body = vec![flags, 0, 0, 0];
        body.extend(u24(width - 1));
        body.extend(u24(height - 1));
        chunk(b"VP8X", &body)
    }

    /// An animation frame of a still's image chunks (ALPH and VP8, or VP8L), at `(x, y)`.
    fn anmf(still: &[u8], (x, y): (u32, u32), flags: u8) -> Vec<u8> {
        let (width, height) = WebPDecoder::new(Cursor::new(still)).unwrap().dimensions();
        let mut body = Vec::new();
        for value in [x / 2, y / 2, width - 1, height - 1, 70] {
            body.extend(u24(value));
        }
        body.push(flags);
        for chunk in chunks_of(still) {
            if [b"ALPH", b"VP8 ", b"VP8L"]
                .iter()
                .any(|tag| chunk.starts_with(*tag))
            {
                body.extend(chunk);
            }
        }
        chunk(b"ANMF", &body)
    }

    /// The files: ImageMagick's (tests/photo-gallery), animations of their stills at offsets and of every kind
    /// (VP8, VP8L, ALPH with VP8), the alpha plane's every filter and header, and damaged copies of each.
    fn files() -> Vec<(String, Vec<u8>, bool)> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/photo-gallery");
        let mut made: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "webp"))
            .map(|path| {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&path).unwrap())
            })
            .collect();
        made.sort();
        assert_eq!(made.len(), 16);
        let still = |name: &str| made.iter().find(|(n, _)| n == name).unwrap().1.clone();

        let mut files: Vec<(String, Vec<u8>, bool)> = Vec::new();
        for (name, bytes) in &made {
            files.push((name.clone(), bytes.clone(), false));
        }
        for (name, bytes) in made.iter().filter(|(n, _)| n.starts_with("anim")) {
            files.push((format!("{name} with a background"), bytes.clone(), true));
        }

        // Animations on a 216×160 canvas: no blend is flag 2, dispose to background flag 1.
        let frames = [
            anmf(&still("lossless_colors200.webp"), (4, 2), 2),
            anmf(&still("lossy_small.webp"), (20, 10), 1),
            anmf(&still("lossy_alpha_filtering1.webp"), (100, 60), 0),
            anmf(&still("lossless_alpha.webp"), (130, 110), 0),
            anmf(&still("lossy_alpha_raw.webp"), (0, 0), 3),
            anmf(&still("lossy_large_strong.webp"), (12, 8), 0),
            anmf(&still("lossless_colors2.webp"), (2, 4), 1),
        ];
        let anim = chunk(b"ANIM", &[200, 100, 50, 255, 2, 0]);
        for (name, flags) in [("animation", 0x12), ("animation without alpha", 0x02)] {
            let mut chunks = vec![vp8x(flags, (216, 160)), anim.clone()];
            chunks.extend(frames.iter().cloned());
            let file = riff(&chunks);
            files.push((name.to_owned(), file.clone(), false));
            files.push((format!("{name} with a background"), file, true));
        }
        let outside = [
            vp8x(0x12, (216, 160)),
            anim.clone(),
            frames[0].clone(),
            anmf(&still("lossy_large_simple.webp"), (20, 20), 0),
        ];
        files.push((
            "a frame outside the canvas".to_owned(),
            riff(&outside),
            false,
        ));

        // The alpha plane, stored raw, with each filter, preprocessed or not, and with invalid headers.
        let raw = chunks_of(&still("lossy_alpha_raw.webp"));
        assert!(raw[1].starts_with(b"ALPH") && raw[1][8] == 0);
        for header in [0x04, 0x08, 0x0c, 0x18, 0x1c, 0x20, 0x02, 0x03, 0x09] {
            let mut alph = raw[1].clone();
            alph[8] = header;
            let file = riff(&[raw[0].clone(), alph, raw[2].clone()]);
            files.push((format!("alpha header {header:#04x}"), file, false));
        }
        let size = u32::from_le_bytes(raw[1][4..8].try_into().unwrap()) as usize;
        let short = chunk(b"ALPH", &raw[1][8..8 + size / 2]);
        let file = riff(&[raw[0].clone(), short, raw[2].clone()]);
        files.push(("a short alpha plane".to_owned(), file, false));

        // Damaged copies: cut short, and with bytes changed past the headers.
        let mut damaged = Vec::new();
        for (name, bytes, background) in &files {
            let len = bytes.len();
            for cut in [len / 3, len / 2, len - len / 5, len - 3] {
                damaged.push((
                    format!("{name} cut at {cut}"),
                    bytes[..cut].to_vec(),
                    *background,
                ));
            }
            let changes = [len / 3, len / 2 + 1, len - len / 4, len - 9];
            for (at, xor) in changes.into_iter().zip([0x5a, 0x01, 0xff, 0x80]) {
                let mut bytes = bytes.clone();
                bytes[at] ^= xor;
                damaged.push((format!("{name} changed at {at}"), bytes, *background));
            }
        }
        files.extend(damaged);
        files
    }

    #[test]
    fn kept_buffers_decode_as_new_ones() {
        let files = files();
        let expected: Vec<Decoded> = files
            .iter()
            .map(|(_, bytes, background)| decode_caught(bytes, Scratch::default(), *background).0)
            .collect();

        // The files decode, fail and panic as expected: these are no vacuous comparisons.
        let count = |f: &dyn Fn(&Decoded) -> bool| expected.iter().filter(|d| f(d)).count();
        let read = count(&|d| matches!(d, Decoded::Read { image: Ok(_), .. }));
        let headers = count(&|d| matches!(d, Decoded::Headers(_)));
        let failed = count(&|d| matches!(d, Decoded::Read { image: Err(_), .. }));
        let whole = |d: &Decoded| match d {
            Decoded::Read {
                animation, frames, ..
            } => *frames == 7 && animation.iter().filter(|f| f.is_ok()).count() == 7,
            _ => false,
        };
        let panicked = count(&|d| matches!(d, Decoded::Panicked));
        let too_large = count(&|d| matches!(d, Decoded::TooLarge));
        println!(
            "{} files: {read} read, {failed} failed, {headers} without headers, {panicked} panicked, \
             {too_large} too large",
            files.len()
        );
        assert!(read > 100 && failed > 20 && headers > 20);
        for (i, (name, ..)) in files.iter().enumerate() {
            if name.starts_with("animation") && !name.contains(" at ") {
                assert!(whole(&expected[i]), "{name}");
            }
        }

        let order: Vec<usize> = (0..files.len()).chain((0..files.len()).rev()).collect();
        for trim in [None, Some(0), Some(4096)] {
            let mut scratch = Scratch::default();
            for &i in &order {
                let (name, bytes, background) = &files[i];
                let (decoded, kept) = decode_caught(bytes, scratch, *background);
                assert!(decoded == expected[i], "{name}, trimmed at {trim:?}");
                scratch = kept;
                if let Some(bytes) = trim {
                    scratch.trim(bytes);
                }
            }
        }
    }
}
