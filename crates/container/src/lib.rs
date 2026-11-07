//! A file's containers, walked without allocating (PLAN §2: the file's own metadata and its pixels' structure):
//! JPEG marker segments (and whether the frame is progressive), the chunks of an extended XMP packet, PNG chunks,
//! RIFF (WebP) chunks and GIF blocks. Each walk yields byte ranges of the file, so the metadata reader and the
//! lossless rewriter share one reading of each format's structure. Model-shaped: no decoding, no XMP parsing.

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
    /// An image: the byte range of its descriptor (from 0x2C), local color table, LZW code size and data.
    Image { range: std::ops::Range<usize> },
}

/// Header and logical screen descriptor length, global color table included.
pub fn gif_header_len(bytes: &[u8]) -> Option<usize> {
    let packed = *bytes.get(10)?;
    let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
    let len = 13 + table;
    (len <= bytes.len()).then_some(len)
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
                let packed = *bytes.get(i + 9)?;
                let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
                let data = i + 10 + table + 1; // LZW minimum code size
                let end = skip_sub_blocks(bytes, data)?;
                (GifBlock::Image { range: i..end }, end)
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
