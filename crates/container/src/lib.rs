//! A file's containers, walked without allocating (PLAN §2: the file's own metadata and its pixels' structure):
//! JPEG marker segments (and whether the frame is progressive), the chunks of an extended XMP packet, PNG chunks,
//! RIFF (WebP) chunks and GIF blocks. Each walk yields byte ranges of the file, so the metadata reader and the
//! lossless rewriter share one reading of each format's structure, and one rule for whether a file is animated
//! ([`PngAnimation`], [`WebpAnimation`]), applied chunk by chunk as a walk goes. The writing half is here too: a PNG
//! chunk and its CRC, a RIFF file and its VP8X canvas, GIF sub-blocks. Model-shaped: no decoding, no XMP parsing.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::ops::Range;

/// The signature of an APP1 segment that holds a chunk of an extended XMP packet.
pub const JPEG_XMP_EXTENSION: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// Where `part`, a slice of `bytes`, is in it.
pub fn range_in(bytes: &[u8], part: &[u8]) -> Range<usize> {
    let start = part.as_ptr().addr() - bytes.as_ptr().addr();
    debug_assert!(start + part.len() <= bytes.len());
    start..start + part.len()
}

/// The `N` bytes of `bytes` at `at`, if they are all in it.
fn bytes_at<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..)?.first_chunk().copied()
}

/// A JPEG marker segment before the image data.
pub struct Segment<'a> {
    pub marker: u8,
    /// Offset of the payload (after the length) in the file.
    pub start: usize,
    pub data: &'a [u8],
}

/// The marker segments up to the start of scan. Embedded JPEGs (EXIF thumbnails, MPF images) are inside these
/// or after the image, so they are never visited. The walk ends at the scan or the end marker, or at damage (see
/// `JpegSegments::damaged`).
pub struct JpegSegments<'a> {
    bytes: &'a [u8],
    i: usize,
    damaged: bool,
}

pub fn jpeg_segments(bytes: &[u8]) -> JpegSegments<'_> {
    JpegSegments { bytes, i: 2, damaged: false }
}

impl JpegSegments<'_> {
    /// Whether the walk stopped at damage (a marker byte that isn't 0xFF, a length that runs past the file, or the
    /// file ending before the scan), rather than at the scan or the end marker.
    pub fn damaged(&self) -> bool {
        self.damaged
    }
}

impl<'a> Iterator for JpegSegments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Segment<'a>> {
        let bytes = self.bytes;
        while let Some([ff, marker, len_hi, len_lo]) = bytes_at::<4>(bytes, self.i) {
            if ff != 0xFF {
                self.damaged = true;
                return None;
            }
            if marker == 0xFF {
                self.i += 1; // fill byte
                continue;
            }
            if marker == 0xDA || marker == 0xD9 {
                return None;
            }
            if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
                self.i += 2;
                continue;
            }
            let len = usize::from(u16::from_be_bytes([len_hi, len_lo]));
            let (start, end) = (self.i + 4, self.i + 2 + len);
            // `None` when the length is under 2 (the range runs backwards) or runs past the file.
            let Some(data) = bytes.get(start..end) else {
                self.damaged = true;
                return None;
            };
            self.i = end;
            return Some(Segment { marker, start, data });
        }
        self.damaged = true;
        None
    }
}

/// A JPEG segment's EXIF block, if it is one.
pub fn jpeg_exif(s: &Segment) -> Option<std::ops::Range<usize>> {
    (s.marker == 0xE1 && s.data.starts_with(b"Exif\0\0")).then(|| s.start + 6..s.start + s.data.len())
}

/// A chunk of an extended XMP packet (Adobe XMP Part 3, 1.1.3.1), as found in a JPEG.
pub struct ExtendedChunk {
    /// Its packet's GUID, in the file.
    pub guid: Range<usize>,
    /// Its packet's length, as this chunk says.
    pub full: usize,
    pub offset: usize,
    pub data: Range<usize>,
    /// The order it was found in.
    pub seq: usize,
}

/// An APP1 segment's data, if it is a chunk of an extended packet.
pub fn add_extended_chunk(bytes: &[u8], data: &[u8], chunks: &mut Vec<ExtendedChunk>) {
    if let Some(rest) = data.strip_prefix(JPEG_XMP_EXTENSION)
        && let Some((guid, rest)) = rest.split_first_chunk::<32>()
        && let Some((&full, rest)) = rest.split_first_chunk::<4>()
        && let Some((&offset, data)) = rest.split_first_chunk::<4>()
    {
        let full = usize::try_from(u32::from_be_bytes(full)).unwrap_or(usize::MAX);
        let offset = usize::try_from(u32::from_be_bytes(offset)).unwrap_or(usize::MAX);
        let (guid, data) = (range_in(bytes, guid), range_in(bytes, data));
        chunks.push(ExtendedChunk { guid, full, offset, data, seq: chunks.len() });
    }
}

pub struct Chunk<'a> {
    pub kind: [u8; 4],
    /// Offset of the chunk's first byte (its length field) in the file.
    pub at: usize,
    /// Total size in the file, header and padding or CRC included.
    pub len: usize,
    pub data: &'a [u8],
}

/// PNG chunks after the signature, up to and including IEND.
pub fn png_chunks(bytes: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let mut i = 8;
    std::iter::from_fn(move || {
        if i + 12 > bytes.len() {
            return None;
        }
        let len = usize::try_from(u32::from_be_bytes(bytes_at::<4>(bytes, i)?)).unwrap_or(usize::MAX);
        let kind: [u8; 4] = bytes_at(bytes, i + 4)?;
        let end = (i + 12).checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        let chunk = Chunk { kind, at: i, len: end - i, data: bytes.get(i + 8..i + 8 + len)? };
        i = if &kind == b"IEND" { bytes.len() } else { end };
        Some(chunk)
    })
}

/// WebP (RIFF) chunks after the `WEBP` form type.
pub fn webp_chunks(bytes: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let end = bytes_at::<4>(bytes, 4)
        .map_or(0, |s| usize::try_from(u32::from_le_bytes(s)).unwrap_or(usize::MAX).saturating_add(8).min(bytes.len()));
    let mut i = 12;
    std::iter::from_fn(move || {
        if i + 8 > end {
            return None;
        }
        let kind: [u8; 4] = bytes_at(bytes, i)?;
        let len = usize::try_from(u32::from_le_bytes(bytes_at::<4>(bytes, i + 4)?)).unwrap_or(usize::MAX);
        let data_end = (i + 8).checked_add(len)?;
        if data_end > end {
            return None;
        }
        let next = (data_end + (len & 1)).min(end);
        let chunk = Chunk { kind, at: i, len: next - i, data: bytes.get(i + 8..data_end)? };
        i = next;
        Some(chunk)
    })
}

/// A PNG chunk of `kind` holding `data`, its CRC (over the type and the data) after it; `None` when `data` is longer
/// than a chunk's length field counts.
pub fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Option<Vec<u8>> {
    let len = u32::try_from(data.len()).ok()?;
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    Some(out)
}

/// Set the CRC of the PNG chunk at `at` in `bytes` (its length field's offset) to what its type and data now are,
/// after its data was changed in place. `None` when the chunk runs past `bytes`.
pub fn fix_png_crc(bytes: &mut [u8], at: usize) -> Option<()> {
    let len = usize::try_from(u32::from_be_bytes(bytes_at::<4>(bytes, at)?)).ok()?;
    let (start, end) = (at.checked_add(4)?, at.checked_add(8)?.checked_add(len)?);
    let crc = crc32fast::hash(bytes.get(start..end)?);
    bytes.get_mut(end..end.checked_add(4)?)?.copy_from_slice(&crc.to_be_bytes());
    Some(())
}

/// A RIFF file of `body` (its form type, such as `WEBP`, then its chunks): the `RIFF` tag and the body's length before
/// it. `None` when the body is longer than the length field counts.
pub fn riff(body: &[u8]) -> Option<Vec<u8>> {
    let len = u32::try_from(body.len()).ok()?;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(body);
    Some(out)
}

/// The fields of a `VP8X` chunk (its flags, then the canvas's width and height less one, 3 bytes each) with the canvas
/// set to `width` × `height` (each at least 1, and below 2²⁴).
pub fn vp8x_with_canvas(fields: [u8; 10], width: u32, height: u32) -> [u8; 10] {
    let mut out = fields;
    out[4..7].copy_from_slice(&width.saturating_sub(1).to_le_bytes()[..3]);
    out[7..10].copy_from_slice(&height.saturating_sub(1).to_le_bytes()[..3]);
    out
}

/// A PNG's animation, as its chunks say in file order ([`PngAnimation::see`] each, as a walk goes). An `acTL` before
/// the first `IDAT` makes it an animation (APNG); one after it is ignored, as an APNG decoder ignores it (the PNG spec
/// puts it first), and the file is a still.
#[derive(Clone, Copy, Debug, Default)]
pub struct PngAnimation {
    image_data: bool,
    /// The `acTL` that counts, with its frame count (`None` for one too short to hold it).
    control: Option<Option<u32>>,
}

impl PngAnimation {
    pub fn see(&mut self, chunk: &Chunk) {
        match &chunk.kind {
            b"IDAT" => self.image_data = true,
            b"acTL" if !self.image_data && self.control.is_none() => {
                self.control = Some(chunk.data.first_chunk::<4>().map(|&n| u32::from_be_bytes(n)));
            }
            _ => {}
        }
    }

    /// Whether the file is an animation.
    pub fn animated(&self) -> bool {
        self.control.is_some()
    }

    /// Its frames: what the `acTL` counts (at least 1), and 1 for a still.
    pub fn frames(&self) -> u32 {
        self.control.flatten().unwrap_or(1).max(1)
    }
}

/// A WebP's animation, as its chunks say ([`WebpAnimation::see`] each, as a walk goes): the `VP8X` chunk's animation
/// flag makes it one, of as many frames as it has `ANMF` chunks.
#[derive(Clone, Copy, Debug, Default)]
pub struct WebpAnimation {
    animated: bool,
    frames: u32,
}

impl WebpAnimation {
    pub fn see(&mut self, chunk: &Chunk) {
        match &chunk.kind {
            b"VP8X" => self.animated |= chunk.data.first().is_some_and(|f| f & 0x02 != 0),
            b"ANMF" => self.frames = self.frames.saturating_add(1),
            _ => {}
        }
    }

    /// Whether the file is an animation.
    pub fn animated(&self) -> bool {
        self.animated
    }

    /// Its frames: its `ANMF` chunks (at least 1) for an animation, and 1 for a still.
    pub fn frames(&self) -> u32 {
        if self.animated { self.frames.max(1) } else { 1 }
    }
}

/// Whether a WebP chunk is lossless image data (`VP8L`).
pub fn webp_lossless(chunk: &Chunk) -> bool {
    &chunk.kind == b"VP8L"
}

/// A PNG chunk's data, as a range of the file.
pub fn chunk_data(c: &Chunk) -> std::ops::Range<usize> {
    c.at + 8..c.at + 8 + c.data.len()
}

/// A WebP EXIF chunk's EXIF block. Some writers keep JPEG's "Exif\0\0" prefix.
pub fn webp_exif(c: &Chunk) -> std::ops::Range<usize> {
    let skip = if c.data.starts_with(b"Exif\0\0") { 6 } else { 0 };
    c.at + 8 + skip..c.at + 8 + c.data.len()
}

/// GIF blocks, as the structure walk sees them.
pub enum GifBlock {
    /// An extension: label and the byte range of the whole block (introducer to terminator).
    Extension { label: u8, range: std::ops::Range<usize> },
    /// An image: the byte range of its descriptor (from 0x2C), local color table, LZW code size and data, and the
    /// descriptor read.
    Image { range: std::ops::Range<usize>, image: GifImage },
}

/// A GIF image's descriptor, as the walk read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GifImage {
    /// Its place on the logical screen.
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    /// The packed byte: the local color table's flag and size, and the interlace flag.
    pub packed: u8,
    /// The local color table's length in bytes (0 for none).
    pub table: usize,
    /// The LZW minimum code size.
    pub code_size: u8,
}

impl GifImage {
    /// Whether its rows are stored interlaced (in four passes).
    pub fn interlaced(&self) -> bool {
        self.packed & 0x40 != 0
    }

    /// The offset of its LZW minimum code size from the descriptor's start (after the local color table).
    pub fn code_at(&self) -> usize {
        10 + self.table
    }
}

/// The length in bytes of the color table that a GIF packed byte (the logical screen's, or an image's) says follows.
pub fn gif_table_len(packed: u8) -> usize {
    if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 }
}

/// Header and logical screen descriptor length, global color table included.
pub fn gif_header_len(bytes: &[u8]) -> Option<usize> {
    let len = 13 + gif_table_len(*bytes.get(10)?);
    (len <= bytes.len()).then_some(len)
}

/// The data of the GIF sub-blocks starting at `at`, joined, and the offset after their terminator. `None` when they
/// run past `bytes`.
pub fn gif_sub_block_data(bytes: &[u8], at: usize) -> Option<(Vec<u8>, usize)> {
    let (mut data, mut i) = (Vec::new(), at);
    loop {
        let n = usize::from(*bytes.get(i)?);
        i += 1;
        if n == 0 {
            return Some((data, i));
        }
        data.extend_from_slice(bytes.get(i..i.checked_add(n)?)?);
        i += n;
    }
}

/// `data` as GIF sub-blocks of at most 255 bytes each, ended by an empty one.
pub fn gif_sub_blocks(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 255 + 1);
    for chunk in data.chunks(255) {
        out.push(u8::try_from(chunk.len()).unwrap_or(u8::MAX));
        out.extend_from_slice(chunk);
    }
    out.push(0);
    out
}

/// Skip data sub-blocks starting at `i`; returns the offset after the terminator.
fn skip_sub_blocks(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let n = usize::from(*bytes.get(i)?);
        i += 1 + n;
        if n == 0 {
            return Some(i);
        }
    }
}

/// The blocks between the header and the trailer, walked as they are read. A GIF is only read as a whole: what
/// the blocks say counts once [`GifBlocks::complete`] says the walk reached the trailer (a truncated download
/// doesn't).
pub fn gif_blocks(bytes: &[u8]) -> GifBlocks<'_> {
    match gif_header_len(bytes) {
        Some(i) => GifBlocks { bytes, i, walk: GifWalk::Blocks },
        None => GifBlocks { bytes, i: 0, walk: GifWalk::Broken },
    }
}

pub struct GifBlocks<'a> {
    bytes: &'a [u8],
    /// The next block's offset.
    i: usize,
    walk: GifWalk,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GifWalk {
    Blocks,
    /// The trailer was reached.
    Complete,
    /// The structure ends before it (truncated) or breaks (an unknown block).
    Broken,
}

impl GifBlocks<'_> {
    /// Whether the walk reached the trailer. Only then do the blocks seen describe the file.
    pub fn complete(&self) -> bool {
        self.walk == GifWalk::Complete
    }

    fn block(&mut self) -> Option<GifBlock> {
        let (bytes, i) = (self.bytes, self.i);
        let (block, end) = match *bytes.get(i)? {
            0x21 => {
                let label = *bytes.get(i + 1)?;
                let end = if label == 0xFF && bytes.get(i + 3..i + 14) == Some(b"XMP DataXMP") {
                    // The XMP packet is stored raw (its bytes double as sub-block lengths), closed by a 258-byte
                    // "magic trailer" ending in 0x00 0x00.
                    let packet_end = find(bytes.get(i + 14..)?, b"<?xpacket end=").map(|p| i + 14 + p)?;
                    let end = find(bytes.get(packet_end..)?, &[0x01, 0xFF]).map(|p| packet_end + p + 258)?;
                    // A block cut short by the end of the file is not a block (the walk ends there, as a truncated
                    // file does): its trailer must be in the file.
                    if end > bytes.len() {
                        return None;
                    }
                    end
                } else {
                    skip_sub_blocks(bytes, i + 2)?
                };
                (GifBlock::Extension { label, range: i..end }, end)
            }
            0x2C => {
                let u16_at = |at: usize| bytes_at::<2>(bytes, at).map(u16::from_le_bytes);
                let packed = *bytes.get(i + 9)?;
                let table = gif_table_len(packed);
                let code_size = *bytes.get(i + 10 + table)?;
                let image = GifImage {
                    left: u16_at(i + 1)?,
                    top: u16_at(i + 3)?,
                    width: u16_at(i + 5)?,
                    height: u16_at(i + 7)?,
                    packed,
                    table,
                    code_size,
                };
                let end = skip_sub_blocks(bytes, i + image.code_at() + 1)?;
                (GifBlock::Image { range: i..end, image }, end)
            }
            0x3B => {
                self.walk = GifWalk::Complete;
                return None;
            }
            _ => return None,
        };
        self.i = end;
        Some(block)
    }
}

impl Iterator for GifBlocks<'_> {
    type Item = GifBlock;

    fn next(&mut self) -> Option<GifBlock> {
        if self.walk != GifWalk::Blocks {
            return None;
        }
        let block = self.block();
        if block.is_none() && self.walk == GifWalk::Blocks {
            self.walk = GifWalk::Broken;
        }
        block
    }
}

/// Whether a JPEG is progressive: its frame header (SOF2) is among the segments before the scan. `None` when those
/// segments are damaged, so the file's structure can't be trusted (the lossless rewriter refuses such a file rather
/// than re-encode it).
pub fn jpeg_is_progressive(bytes: &[u8]) -> Option<bool> {
    let mut segments = jpeg_segments(bytes);
    if segments.by_ref().any(|s| s.marker == 0xC2) {
        return Some(true);
    }
    (!segments.damaged()).then_some(false)
}

/// Where `needle` first is in `haystack`.
pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG of these chunks after its signature, each with its CRC.
    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            out.extend(png_chunk(kind, data).unwrap());
        }
        out
    }

    fn png_animation(bytes: &[u8]) -> PngAnimation {
        let mut a = PngAnimation::default();
        png_chunks(bytes).for_each(|c| a.see(&c));
        a
    }

    // An `acTL` before the image data makes an animation of its frames; one after it is ignored: a still, to the
    // metadata's frame count and to the edit's check alike.
    #[test]
    fn an_actl_after_the_image_data_is_a_still() {
        let actl = [0, 0, 0, 3, 0, 0, 0, 0];
        let before = png(&[(b"IHDR", &[0; 13]), (b"acTL", &actl), (b"IDAT", &[0]), (b"IEND", &[])]);
        let after = png(&[(b"IHDR", &[0; 13]), (b"IDAT", &[0]), (b"acTL", &actl), (b"IEND", &[])]);
        let (a, b) = (png_animation(&before), png_animation(&after));
        assert!(a.animated() && a.frames() == 3);
        assert!(!b.animated() && b.frames() == 1);
    }

    #[test]
    fn a_fixed_crc_is_the_chunks_own() {
        let mut bytes = png(&[(b"eXIf", b"abcd"), (b"IEND", &[])]);
        bytes[8 + 8] = b'z';
        fix_png_crc(&mut bytes, 8).unwrap();
        assert_eq!(bytes[8..8 + 16], png_chunk(b"eXIf", b"zbcd").unwrap()[..]);
    }

    #[test]
    fn sub_blocks_read_back_as_written() {
        let data: Vec<u8> = (0..600u32).map(|i| u8::try_from(i % 251).unwrap()).collect();
        let blocks = gif_sub_blocks(&data);
        assert_eq!(gif_sub_block_data(&blocks, 0), Some((data, blocks.len())));
    }

    /// A JPEG's start, then each segment (marker and payload) with its length, then a scan and the end marker.
    fn jpeg(segments: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        for &(marker, payload) in segments {
            out.extend([0xFF, marker]);
            out.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
            out.extend(payload);
        }
        out.extend([0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        out
    }

    #[test]
    fn a_jpeg_is_progressive_when_its_frame_header_is_sof2() {
        let frame = [8, 0, 8, 0, 8, 3];
        assert_eq!(jpeg_is_progressive(&jpeg(&[(0xC0, &frame)])), Some(false));
        assert_eq!(jpeg_is_progressive(&jpeg(&[(0xE0, &[0; 4]), (0xC2, &frame)])), Some(true));
    }

    #[test]
    fn a_damaged_jpeg_is_not_classified() {
        // A segment whose length runs past the file.
        assert_eq!(jpeg_is_progressive(&[0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, 1, 2, 3]), None);
        // A marker byte where a marker must be.
        assert_eq!(jpeg_is_progressive(&[0xFF, 0xD8, 0x00, 0x00, 0x00, 0x02]), None);
        // The file ends before the scan.
        assert_eq!(jpeg_is_progressive(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x02]), None);
    }
}
