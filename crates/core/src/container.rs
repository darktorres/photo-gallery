//! A file's containers, walked without allocating (PLAN §2: the file's own metadata and its pixels' structure):
//! JPEG marker segments (and whether the frame is progressive), the chunks of an extended XMP packet, PNG chunks,
//! RIFF (WebP) chunks and GIF blocks. Each walk yields byte ranges of the file, so the metadata reader and the
//! lossless rewriter share one reading of each format's structure. Model-shaped: no decoding, no XMP parsing.

use std::ops::Range;

/// The signature of an APP1 segment that holds a chunk of an extended XMP packet.
pub(crate) const JPEG_XMP_EXTENSION: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// Where `part`, a slice of `bytes`, is in it.
pub(crate) fn range_in(bytes: &[u8], part: &[u8]) -> Range<usize> {
    let start = part.as_ptr() as usize - bytes.as_ptr() as usize;
    debug_assert!(start + part.len() <= bytes.len());
    start..start + part.len()
}

/// A JPEG marker segment before the image data.
pub(crate) struct Segment<'a> {
    pub(crate) marker: u8,
    /// Offset of the payload (after the length) in the file.
    pub(crate) start: usize,
    pub(crate) data: &'a [u8],
}

/// The marker segments up to the start of scan. Embedded JPEGs (EXIF thumbnails, MPF images) are inside these
/// or after the image, so they are never visited.
pub(crate) fn jpeg_segments(bytes: &[u8]) -> impl Iterator<Item = Segment<'_>> {
    let mut i = 2;
    std::iter::from_fn(move || {
        while i + 4 <= bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            if marker == 0xFF {
                i += 1; // fill byte
                continue;
            }
            if marker == 0xDA || marker == 0xD9 {
                return None;
            }
            if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            let (start, end) = (i + 4, i + 2 + len);
            if len < 2 || end > bytes.len() {
                return None;
            }
            i = end;
            return Some(Segment { marker, start, data: &bytes[start..end] });
        }
        None
    })
}

/// A JPEG segment's EXIF block, if it is one.
pub(crate) fn jpeg_exif(s: &Segment) -> Option<std::ops::Range<usize>> {
    (s.marker == 0xE1 && s.data.starts_with(b"Exif\0\0")).then(|| s.start + 6..s.start + s.data.len())
}

/// A chunk of an extended XMP packet (Adobe XMP Part 3, 1.1.3.1), as found in a JPEG.
pub(crate) struct ExtendedChunk {
    /// Its packet's GUID, in the file.
    pub(crate) guid: Range<usize>,
    /// Its packet's length, as this chunk says.
    pub(crate) full: usize,
    pub(crate) offset: usize,
    pub(crate) data: Range<usize>,
    /// The order it was found in.
    pub(crate) seq: usize,
}

/// An APP1 segment's data, if it is a chunk of an extended packet.
pub(crate) fn add_extended_chunk(bytes: &[u8], data: &[u8], chunks: &mut Vec<ExtendedChunk>) {
    if let Some(rest) = data.strip_prefix(JPEG_XMP_EXTENSION)
        && rest.len() >= 40
    {
        let full = u32::from_be_bytes(rest[32..36].try_into().unwrap()) as usize;
        let offset = u32::from_be_bytes(rest[36..40].try_into().unwrap()) as usize;
        let (guid, data) = (range_in(bytes, &rest[..32]), range_in(bytes, &rest[40..]));
        chunks.push(ExtendedChunk { guid, full, offset, data, seq: chunks.len() });
    }
}

pub(crate) struct Chunk<'a> {
    pub kind: [u8; 4],
    /// Offset of the chunk's first byte (its length field) in the file.
    pub at: usize,
    /// Total size in the file, header and padding or CRC included.
    pub len: usize,
    pub data: &'a [u8],
}

/// PNG chunks after the signature, up to and including IEND.
pub(crate) fn png_chunks(bytes: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let mut i = 8;
    std::iter::from_fn(move || {
        if i + 12 > bytes.len() {
            return None;
        }
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
        let end = (i + 12).checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        let chunk = Chunk { kind, at: i, len: end - i, data: &bytes[i + 8..i + 8 + len] };
        i = if &kind == b"IEND" { bytes.len() } else { end };
        Some(chunk)
    })
}

/// WebP (RIFF) chunks after the `WEBP` form type.
pub(crate) fn webp_chunks(bytes: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let end = bytes.get(4..8).map_or(0, |s| (u32::from_le_bytes(s.try_into().unwrap()) as usize + 8).min(bytes.len()));
    let mut i = 12;
    std::iter::from_fn(move || {
        if i + 8 > end {
            return None;
        }
        let kind: [u8; 4] = bytes[i..i + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        let data_end = (i + 8).checked_add(len)?;
        if data_end > end {
            return None;
        }
        let next = (data_end + (len & 1)).min(end);
        let chunk = Chunk { kind, at: i, len: next - i, data: &bytes[i + 8..data_end] };
        i = next;
        Some(chunk)
    })
}

/// A PNG chunk's data, as a range of the file.
pub(crate) fn chunk_data(c: &Chunk) -> std::ops::Range<usize> {
    c.at + 8..c.at + 8 + c.data.len()
}

/// A WebP EXIF chunk's EXIF block. Some writers keep JPEG's "Exif\0\0" prefix.
pub(crate) fn webp_exif(c: &Chunk) -> std::ops::Range<usize> {
    let skip = if c.data.starts_with(b"Exif\0\0") { 6 } else { 0 };
    c.at + 8 + skip..c.at + 8 + c.data.len()
}

/// GIF blocks, as the structure walk sees them.
pub(crate) enum GifBlock {
    /// An extension: label and the byte range of the whole block (introducer to terminator).
    Extension { label: u8, range: std::ops::Range<usize> },
    /// An image: the byte range of its descriptor (from 0x2C), local color table, LZW code size and data.
    Image { range: std::ops::Range<usize> },
}

/// Header and logical screen descriptor length, global color table included.
pub(crate) fn gif_header_len(bytes: &[u8]) -> Option<usize> {
    let packed = *bytes.get(10)?;
    let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
    let len = 13 + table;
    (len <= bytes.len()).then_some(len)
}

/// Skip data sub-blocks starting at `i`; returns the offset after the terminator.
fn skip_sub_blocks(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let n = *bytes.get(i)? as usize;
        i += 1 + n;
        if n == 0 {
            return Some(i);
        }
    }
}

/// The blocks between the header and the trailer, walked as they are read. A GIF is only read as a whole: what
/// the blocks say counts once [`GifBlocks::complete`] says the walk reached the trailer (a truncated download
/// doesn't).
pub(crate) fn gif_blocks(bytes: &[u8]) -> GifBlocks<'_> {
    match gif_header_len(bytes) {
        Some(i) => GifBlocks { bytes, i, walk: GifWalk::Blocks },
        None => GifBlocks { bytes, i: 0, walk: GifWalk::Broken },
    }
}

pub(crate) struct GifBlocks<'a> {
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
    pub(crate) fn complete(&self) -> bool {
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
                    let packet_end = find(&bytes[i + 14..], b"<?xpacket end=").map(|p| i + 14 + p)?;
                    find(&bytes[packet_end..], &[0x01, 0xFF]).map(|p| packet_end + p + 258)?
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

/// Whether a JPEG is progressive: its frame header (SOF2) is among the segments before the scan.
pub(crate) fn jpeg_is_progressive(bytes: &[u8]) -> bool {
    jpeg_segments(bytes).any(|s| s.marker == 0xC2)
}

/// Where `needle` first is in `haystack`.
pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
