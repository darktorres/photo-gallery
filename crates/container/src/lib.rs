//! A file's containers, walked without allocating (the file's own metadata and its pixels' structure):
//! JPEG marker segments (and whether the frame is progressive), the chunks of an extended XMP packet, PNG chunks,
//! RIFF (WebP) chunks and GIF blocks. Each walk yields the file's structure as byte ranges (PNG chunks yield their chunk values), so the metadata reader and the
//! lossless rewriter share one reading of each format's structure, and one rule for whether a file is animated
//! ([`PngAnimation`], [`WebpAnimation`]), applied chunk by chunk as a walk goes. The writing half is here too: a PNG
//! chunk and its CRC, a RIFF file and its VP8X canvas, GIF sub-blocks. Model-shaped: no decoding, no XMP parsing.

use std::io::{self, Read, Seek, SeekFrom};
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

    /// Where the walk stopped at a start-of-scan marker (its 0xFF byte), once the walk is over and undamaged. `None`
    /// at the end marker, at damage, or when the file ends before any scan.
    pub fn scan_at(&self) -> Option<usize> {
        let at = self.i;
        (!self.damaged && bytes_at::<2>(self.bytes, at) == Some([0xFF, 0xDA])).then_some(at)
    }
}

/// What the marker after a 0xFF byte is, by its code: a fill byte, a standalone marker (restart, TEM), the end of the
/// image, or a segment with its code and its length (the scan header is a segment, code 0xDA).
enum Marker {
    Fill,
    Standalone,
    End,
    Segment { code: u8, len: usize },
}

fn marker_kind(code: u8, hi: u8, lo: u8) -> Marker {
    match code {
        0xFF => Marker::Fill,
        0xD0..=0xD7 | 0x01 => Marker::Standalone,
        0xD9 => Marker::End,
        _ => Marker::Segment { code, len: usize::from(u16::from_be_bytes([hi, lo])) },
    }
}

impl<'a> Iterator for JpegSegments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Segment<'a>> {
        let bytes = self.bytes;
        while let Some([ff, code, hi, lo]) = bytes_at::<4>(bytes, self.i) {
            if ff != 0xFF {
                self.damaged = true;
                return None;
            }
            match marker_kind(code, hi, lo) {
                Marker::Fill => self.i += 1,
                Marker::Standalone => self.i += 2,
                Marker::End | Marker::Segment { code: 0xDA, .. } => return None,
                Marker::Segment { code, len } => {
                    let (start, end) = (self.i + 4, self.i + 2 + len);
                    // `None` when the length is under 2 (the range runs backwards) or runs past the file.
                    let Some(data) = bytes.get(start..end) else {
                        self.damaged = true;
                        return None;
                    };
                    self.i = end;
                    return Some(Segment { marker: code, start, data });
                }
            }
        }
        self.damaged = true;
        None
    }
}

/// A JPEG segment's EXIF block, if it is one.
pub fn jpeg_exif(s: &Segment) -> Option<std::ops::Range<usize>> {
    (s.marker == 0xE1 && s.data.starts_with(b"Exif\0\0")).then(|| s.start + 6..s.start + s.data.len())
}

/// The most bytes a decode takes from one file, the one cap the containers and the metadata share: 16 MiB. Past it, a
/// gather gives up and the file is read whole, and a compressed packet that would inflate past what is left is absent
/// (`pg_metadata`).
pub const DECODE_CAP: u64 = 16 << 20;

/// Most bytes [`jpeg_import_bytes`] and [`png_import_bytes`] read of a file: past them, the file is read whole. This is
/// [`DECODE_CAP`].
pub const GATHER_CAP: u64 = DECODE_CAP;

/// The bytes the import's reads take at a time: a JPEG's first read (its header, as a rule), a PNG's head and tail.
/// Measured, cold: about as fast as 4 or 32 KiB on an NVMe disk, and as 64 KiB on a hard disk.
const READ_BLOCK: usize = 16 << 10;

/// What a JPEG's first bytes tell of its header: its marker segments up to and through the scan header, all that the
/// metadata reader and the decoders' header reads look at (nothing past the scan), so the file's first bytes up to the
/// header's end give the whole file's facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JpegHeader {
    /// The header ends here (with the scan header).
    Ends(usize),
    /// The bytes say no more until there are this many of them.
    Needs(usize),
    /// Not a JPEG, or not one whose header ends in a scan: a marker byte that isn't 0xFF, a segment length under 2, the
    /// end marker before a scan, no frame header before the scan, or a scan header under 8 bytes.
    Unread,
}

/// Where the JPEG header in `bytes` ends: an undamaged marker walk from the start of image that reaches the start of
/// scan, a frame header before it, and the whole scan header (its length counts itself and at least one component: 8
/// or more).
pub fn jpeg_header_end(bytes: &[u8]) -> JpegHeader {
    match bytes_at::<3>(bytes, 0) {
        None => return JpegHeader::Needs(3),
        Some([0xFF, 0xD8, 0xFF]) => {}
        Some(_) => return JpegHeader::Unread,
    }
    let mut i = 2;
    let mut frame = false;
    loop {
        let Some([ff, code, hi, lo]) = bytes_at::<4>(bytes, i) else { return JpegHeader::Needs(i + 4) };
        if ff != 0xFF {
            return JpegHeader::Unread;
        }
        let (code, len) = match marker_kind(code, hi, lo) {
            Marker::Fill => {
                i += 1;
                continue;
            }
            Marker::Standalone => {
                i += 2;
                continue;
            }
            Marker::End => return JpegHeader::Unread,
            Marker::Segment { code, len } => (code, len),
        };
        if len < 2 {
            return JpegHeader::Unread;
        }
        let end = i + 2 + len;
        if code == 0xDA {
            if !frame || len < 8 {
                return JpegHeader::Unread;
            }
            return if end <= bytes.len() { JpegHeader::Ends(end) } else { JpegHeader::Needs(end) };
        }
        // A start-of-frame marker: C0..=CF, but not C4 (DHT), C8 (JPG) or CC (DAC).
        frame |= matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC4 | 0xC8 | 0xCC);
        i = end;
    }
}

/// The JPEG's bytes that the import reads: the file's first bytes, through at least the end of its header
/// ([`jpeg_header_end`]), so its image data needn't be read. A 16 KiB block first, then on to where the header's
/// segment lengths say it goes (a large metadata segment is read through), a block at a time past each. Measured, cold,
/// on 2,852 wallpaper JPEGs: 11,264 files/s against 1,725 for a fixed 256 KiB prefix (8 reads in flight on an NVMe disk).
///
/// `None` when the file is not a JPEG or its header doesn't end in a scan ([`JpegHeader::Unread`]), or the header runs
/// past the file or [`GATHER_CAP`]: the whole file is then read.
pub fn jpeg_import_bytes<R: Read + Seek>(file: &mut R) -> io::Result<Option<Vec<u8>>> {
    let size = usize::try_from(file.seek(SeekFrom::End(0))?).unwrap_or(usize::MAX);
    let cap = usize::try_from(GATHER_CAP).unwrap_or(usize::MAX);
    let mut file = Positioned { file, size };
    let mut bytes = file.read(0, size.min(READ_BLOCK))?;
    loop {
        match jpeg_header_end(&bytes) {
            JpegHeader::Ends(_) => return Ok(Some(bytes)),
            JpegHeader::Needs(n) if n <= size && n <= cap => {
                let to = n.saturating_add(READ_BLOCK).min(size);
                let more = file.read(bytes.len(), to - bytes.len())?;
                bytes.extend_from_slice(&more);
            }
            JpegHeader::Needs(_) | JpegHeader::Unread => return Ok(None),
        }
    }
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

/// Adds an APP1 segment's data to `chunks`, if it is a chunk of an extended packet.
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

const PNG_SIGNATURE: [u8; 8] = *b"\x89PNG\r\n\x1a\n";
/// An `IEND` chunk: no data, and its CRC.
const PNG_IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
/// Room for the chunks after the image data when the tail is read again, wider.
const PNG_SLACK: usize = 64 << 10;
/// The widest tail read.
const PNG_TAIL_CAP: usize = 4 << 20;
/// The most runs of image data chunks of one length [`png_import_bytes`] searches through.
const PNG_RUNS: usize = 16;

/// The PNG's bytes that the import reads, as [`png_import_bytes_of`] makes them of the whole file, read from the file
/// at its two ends, so an image's data needn't be read: the signature; the chunks before the first image data, as they
/// are; a zero-length `IDAT` in the image data's place (a decoder's header read stops there, as it does in the whole
/// file); and the chunks after the image data up to the first `IEND`, as they are. The chunks metadata reads come in the
/// order the whole file has them, so the import's facts are the whole file's.
///
/// An encoder writes the image data as consecutive `IDAT` chunks of one length (the first and the last ones may be
/// shorter), so the chunks start on a grid that the first two give. The chunks after the image data are found back from
/// the `IEND` at the end of the file, each checked by its CRC, until the last `IDAT`; where the image data ends is
/// checked on the grid (a chunk's header read there), which ties it to this image's data rather than to image data
/// appended after the file's first `IEND` (unless that data's chunks happen to fall on the same grid).
/// When the tail doesn't tell, a binary search of the grid's headers finds the image data's end, and the chunks after it
/// are read on to the first `IEND`. A head and a tail read for most files, a few more for the rest: the reads are
/// 16 KiB blocks at the two ends, and chunk headers.
///
/// `None` when the file is not a PNG, has no image data, is animated, its image data chunks are not of one length, a
/// chunk runs past the end, or the gathered bytes would pass [`GATHER_CAP`]: the whole file is then read.
pub fn png_import_bytes<R: Read + Seek>(file: &mut R) -> io::Result<Option<Vec<u8>>> {
    let size = usize::try_from(file.seek(SeekFrom::End(0))?).unwrap_or(usize::MAX);
    let cap = usize::try_from(GATHER_CAP).unwrap_or(usize::MAX);
    let mut file = Positioned { file, size };
    if size < PNG_SIGNATURE.len() {
        return Ok(None);
    }
    let mut head = file.read(0, size.min(READ_BLOCK))?;
    if head.get(..PNG_SIGNATURE.len()) != Some(PNG_SIGNATURE.as_slice()) {
        return Ok(None);
    }
    // The chunks before the image data, the head read on when they pass it.
    let mut at = PNG_SIGNATURE.len();
    let (idat_at, idat_len) = loop {
        let header_end = at.saturating_add(8);
        if header_end > head.len() {
            if header_end > size || header_end > cap {
                return Ok(None);
            }
            let to = header_end.saturating_add(READ_BLOCK).min(size);
            let more = file.read(head.len(), to - head.len())?;
            head.extend_from_slice(&more);
        }
        let Some((len, kind)) = png_header_in(&head, at) else { return Ok(None) };
        match &kind {
            b"IDAT" => break (at, len),
            b"IEND" | b"acTL" | b"fdAT" => return Ok(None),
            _ => {}
        }
        at = match png_chunk_end(at, len) {
            Some(end) if end <= size => end,
            _ => return Ok(None),
        };
    };
    let mut out = head.get(..idat_at).unwrap_or_default().to_vec();
    png_zero_idat(&mut out);
    let Some(first_end) = png_chunk_end(idat_at, idat_len).filter(|&end| end.saturating_add(12) <= size) else {
        return Ok(None);
    };
    // The second chunk's header gives the grid: in the head, read on its own when it is before the tail, or in the tail.
    let tail_at = size.saturating_sub(READ_BLOCK).max(first_end);
    let mut second = png_header_in(&head, first_end);
    if second.is_none() && first_end.saturating_add(8) <= tail_at {
        second = file.header(first_end)?;
    }
    let tail = file.read(tail_at, size - tail_at)?;
    let mut known = Known { head, tail, tail_at };
    let second = match second.or_else(|| known.header(first_end)) {
        Some(header) => Some(header),
        None => file.header(first_end)?,
    };
    let grid = match second {
        Some((len, kind)) if &kind == b"IDAT" => Grid { first_end, at: first_end, len },
        _ => Grid { first_end, at: idat_at, len: idat_len },
    };
    let after = match png_tail_chunks(&mut file, &mut known, &grid, idat_len)? {
        Some(after) => after,
        None => match png_grid_search(&mut file, &grid, cap)? {
            Some(after) => after,
            None => return Ok(None),
        },
    };
    if out.len().saturating_add(after.len()) > cap {
        return Ok(None);
    }
    out.extend_from_slice(&after);
    Ok(Some(out))
}

/// The PNG's bytes that the import reads, of the whole file in memory: the signature; the chunks before the first image
/// data, as they are; a zero-length `IDAT` in the image data's place; and the chunks after the image data up to `IEND`,
/// without the image data (`IDAT`, and an animation's `fdAT`), as [`png_chunks`] walks them (a chunk that runs past the
/// end ends the walk). `None` when the bytes are not a PNG, have no image data, or the gathered bytes would pass
/// [`GATHER_CAP`]. [`png_import_bytes`] gives these bytes, or none, from the file.
pub fn png_import_bytes_of(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..PNG_SIGNATURE.len()) != Some(PNG_SIGNATURE.as_slice()) {
        return None;
    }
    let cap = usize::try_from(GATHER_CAP).unwrap_or(usize::MAX);
    let mut out = PNG_SIGNATURE.to_vec();
    let mut image_data = false;
    for chunk in png_chunks(bytes) {
        match &chunk.kind {
            b"IDAT" if !image_data => {
                image_data = true;
                png_zero_idat(&mut out);
            }
            b"IDAT" | b"fdAT" => {}
            _ => {
                if out.len().saturating_add(chunk.len) > cap {
                    return None;
                }
                out.extend_from_slice(bytes.get(chunk.at..chunk.at + chunk.len)?);
            }
        }
    }
    image_data.then_some(out)
}

fn png_zero_idat(out: &mut Vec<u8>) {
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&crc32fast::hash(b"IDAT").to_be_bytes());
}

/// The end of the chunk at `at` with `len` bytes of data (its length, type and CRC around them).
fn png_chunk_end(at: usize, len: usize) -> Option<usize> {
    at.checked_add(12)?.checked_add(len)
}

/// The length and type of the chunk at `at` of `bytes`, when its header is in them.
fn png_header_in(bytes: &[u8], at: usize) -> Option<(usize, [u8; 4])> {
    let len = usize::try_from(u32::from_be_bytes(bytes_at::<4>(bytes, at)?)).unwrap_or(usize::MAX);
    Some((len, bytes_at::<4>(bytes, at.checked_add(4)?)?))
}

/// A file read at offsets.
struct Positioned<'a, R> {
    file: &'a mut R,
    size: usize,
}

impl<R: Read + Seek> Positioned<'_, R> {
    fn read(&mut self, at: usize, len: usize) -> io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(u64::try_from(at).unwrap_or(u64::MAX)))?;
        let mut bytes = vec![0; len];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// The chunk header at `at`, by a read of it; `None` past the end.
    fn header(&mut self, at: usize) -> io::Result<Option<(usize, [u8; 4])>> {
        if at.saturating_add(8) > self.size {
            return Ok(None);
        }
        Ok(png_header_in(&self.read(at, 8)?, 0))
    }
}

/// What the gather has read of a file: its head (from the start) and its tail (from `tail_at` to the end).
struct Known {
    head: Vec<u8>,
    tail: Vec<u8>,
    tail_at: usize,
}

impl Known {
    fn header(&self, at: usize) -> Option<(usize, [u8; 4])> {
        png_header_in(&self.head, at).or_else(|| png_header_in(&self.tail, at.checked_sub(self.tail_at)?))
    }

    /// The chunk header at `at`, from what is read, else by a read of it.
    fn header_or_read<R: Read + Seek>(
        &self,
        file: &mut Positioned<'_, R>,
        at: usize,
    ) -> io::Result<Option<(usize, [u8; 4])>> {
        match self.header(at) {
            Some(header) => Ok(Some(header)),
            None => file.header(at),
        }
    }
}

/// Where a PNG's image data chunks are: the first one's end, and the grid of the others (where the first of them starts,
/// and their length; for one chunk, the first one's).
struct Grid {
    first_end: usize,
    at: usize,
    len: usize,
}

impl Grid {
    fn step(&self) -> usize {
        self.len.saturating_add(12)
    }

    fn on(&self, at: usize) -> bool {
        at >= self.at && (at - self.at).is_multiple_of(self.step())
    }
}

/// What a tail tells of the chunks after the image data.
enum AfterImage {
    /// Their ranges in the tail, in order, up to the first `IEND`.
    Found(Vec<Range<usize>>),
    /// The tail does not reach back to where the image data ends: the chunks found back from `IEND` (the last first),
    /// and where the earliest of them (or of the short `IDAT`s before them) starts in the tail.
    Short(Vec<Range<usize>>, usize),
    /// Not as the gather reads a PNG: the whole file is read.
    Unread,
}

/// The chunks after the image data in `tail`, the file's bytes from `start` to its end. When the tail starts right after
/// the first `IDAT`, they are walked forward; else they are found back from the `IEND` at the end, each chunk ending
/// where the next starts and its CRC holding, until an `IDAT` on the grid (the last full chunk, or the short last one
/// after them; short ones off the grid before it are passed over).
fn after_image(tail: &[u8], start: usize, grid: &Grid) -> AfterImage {
    let mut chunks = Vec::new();
    if start == grid.first_end {
        let mut i = 0;
        loop {
            let Some((len, kind)) = png_header_in(tail, i) else { return AfterImage::Unread };
            let Some(end) = png_chunk_end(i, len).filter(|&end| end <= tail.len()) else { return AfterImage::Unread };
            match &kind {
                b"IDAT" => {}
                b"fdAT" => return AfterImage::Unread,
                _ => {
                    chunks.push(i..end);
                    if &kind == b"IEND" {
                        return AfterImage::Found(chunks);
                    }
                }
            }
            i = end;
        }
    }
    let Some(iend_at) = tail.len().checked_sub(PNG_IEND.len()) else { return AfterImage::Unread };
    if tail.get(iend_at..) != Some(PNG_IEND.as_slice()) {
        return AfterImage::Unread;
    }
    chunks.push(iend_at..tail.len());
    let mut next = iend_at;
    let mut in_image_data = false;
    loop {
        let Some(p) = png_chunk_ending_at(tail, next) else { return AfterImage::Short(chunks, next) };
        let Some((len, kind)) = png_header_in(tail, p) else { return AfterImage::Unread };
        match &kind {
            b"IDAT" => {
                let at = start + p;
                if len > grid.len || at < grid.at {
                    return AfterImage::Unread;
                }
                if grid.on(at) {
                    chunks.reverse();
                    return AfterImage::Found(chunks);
                }
                if len == grid.len {
                    return AfterImage::Unread;
                }
                in_image_data = true;
                next = p;
                continue;
            }
            // A chunk between image data chunks, or an animation's.
            _ if in_image_data => return AfterImage::Unread,
            b"fdAT" => return AfterImage::Unread,
            // The walk ends at the first `IEND`: what follows it is not the image's.
            b"IEND" => chunks.clear(),
            _ => {}
        }
        chunks.push(p..next);
        next = p;
    }
}

/// Where the chunk that ends at `end` of `tail` starts: its length reaches `end`, its type is letters and its CRC holds.
fn png_chunk_ending_at(tail: &[u8], end: usize) -> Option<usize> {
    let crc = u32::from_be_bytes(bytes_at::<4>(tail, end.checked_sub(4)?)?);
    (0..=end.checked_sub(12)?).rev().find(|&p| {
        png_header_in(tail, p).is_some_and(|(len, kind)| {
            png_chunk_end(p, len) == Some(end)
                && kind.iter().all(u8::is_ascii_alphabetic)
                && tail.get(p + 4..end - 4).is_some_and(|typed| crc32fast::hash(typed) == crc)
        })
    })
}

/// Whether the image data ends at `end` (a file offset), by the grid: its last chunk starts on the grid point before
/// `end` (the chunks before it are full), or is a short one after a short one there.
fn png_ends_on_grid<R: Read + Seek>(
    file: &mut Positioned<'_, R>,
    known: &Known,
    grid: &Grid,
    end: usize,
) -> io::Result<bool> {
    let step = grid.step();
    let Some(before) = end.checked_sub(12).and_then(|e| e.checked_sub(grid.at)) else { return Ok(false) };
    let point = grid.at + before / step * step;
    let Some((len, kind)) = known.header_or_read(file, point)? else { return Ok(false) };
    if &kind != b"IDAT" || len > grid.len {
        return Ok(false);
    }
    let Some(next) = png_chunk_end(point, len) else { return Ok(false) };
    if next == end {
        return Ok(true);
    }
    if len == grid.len || next.saturating_add(12) > end {
        return Ok(false);
    }
    Ok(known
        .header_or_read(file, next)?
        .is_some_and(|(len, kind)| &kind == b"IDAT" && len < grid.len && png_chunk_end(next, len) == Some(end)))
}

/// The chunks after the image data, from the tail (read wider once when it is short); `None` when the tail doesn't tell.
fn png_tail_chunks<R: Read + Seek>(
    file: &mut Positioned<'_, R>,
    known: &mut Known,
    grid: &Grid,
    idat_len: usize,
) -> io::Result<Option<Vec<u8>>> {
    // Room for the chunks after the image data, short chunks at its end, and the last full one.
    let wide = idat_len.max(grid.len).saturating_mul(2).saturating_add(24 + PNG_SLACK).min(PNG_TAIL_CAP);
    let wide_at = file.size.saturating_sub(wide).max(grid.first_end).min(known.tail_at);
    loop {
        let found = match after_image(&known.tail, known.tail_at, grid) {
            AfterImage::Found(chunks) => chunks,
            AfterImage::Unread => return Ok(None),
            AfterImage::Short(mut chunks, next) => {
                if png_ends_on_grid(file, known, grid, known.tail_at + next)? {
                    chunks.reverse();
                    chunks
                } else if known.tail_at > wide_at {
                    let mut wider = file.read(wide_at, known.tail_at - wide_at)?;
                    wider.extend_from_slice(&known.tail);
                    known.tail = wider;
                    known.tail_at = wide_at;
                    continue;
                } else {
                    return Ok(None);
                }
            }
        };
        let mut after = Vec::new();
        for range in found {
            after.extend_from_slice(known.tail.get(range).unwrap_or_default());
        }
        return Ok(Some(after));
    }
}

/// The chunks after the image data, found forward through the grid: the last full chunk of a run of them by a binary
/// search of their headers, the short chunks after it (a full one after them starts a new run), then the chunks up to
/// the first `IEND`, read as they are. `None` when the image data's chunks are not of one length, or a chunk runs past
/// the end or the cap.
fn png_grid_search<R: Read + Seek>(
    file: &mut Positioned<'_, R>,
    grid: &Grid,
    cap: usize,
) -> io::Result<Option<Vec<u8>>> {
    let (full, step) = (grid.len, grid.step());
    let mut run_at = grid.at;
    let mut at;
    let mut runs = 0;
    'runs: loop {
        // The run's chunks are full up to some index, and not after it.
        let (mut lo, mut hi) = (0usize, (file.size - run_at) / step + 1);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if file.header(run_at + mid * step)?.is_some_and(|(len, kind)| &kind == b"IDAT" && len == full) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        at = run_at + (lo + 1) * step;
        loop {
            let Some((len, kind)) = file.header(at)? else { return Ok(None) };
            if &kind != b"IDAT" {
                break 'runs;
            }
            if len > full {
                return Ok(None);
            }
            if len == full {
                break;
            }
            let Some(end) = png_chunk_end(at, len) else { return Ok(None) };
            at = end;
        }
        runs += 1;
        if runs > PNG_RUNS {
            return Ok(None);
        }
        run_at = at;
    }
    let mut after = Vec::new();
    loop {
        let Some((len, kind)) = file.header(at)? else { return Ok(None) };
        if &kind == b"IDAT" || &kind == b"fdAT" {
            return Ok(None);
        }
        let Some(end) = png_chunk_end(at, len).filter(|&end| end <= file.size) else { return Ok(None) };
        if after.len().saturating_add(end - at) > cap {
            return Ok(None);
        }
        after.extend_from_slice(&file.read(at, end - at)?);
        if &kind == b"IEND" {
            return Ok(Some(after));
        }
        at = end;
    }
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
        let mut out = PNG_SIGNATURE.to_vec();
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

    /// A JPEG header with these segments (marker, payload length) before a frame header and a scan header, then
    /// `entropy` bytes of image data and the end marker.
    fn jpeg_of(segments: &[(u8, usize)], entropy: usize) -> Vec<u8> {
        let frame = [8, 0, 8, 0, 8, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        let mut out = vec![0xFF, 0xD8];
        let mut add = |marker: u8, payload: &[u8]| {
            out.extend([0xFF, marker]);
            out.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
            out.extend(payload);
        };
        for &(marker, len) in segments {
            add(marker, &noise(len, u64::from(marker)));
        }
        add(0xC0, &frame);
        add(0xDA, &[3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3F, 0]);
        out.extend(noise(entropy, 7).into_iter().map(|b| if b == 0xFF { 0 } else { b }));
        out.extend([0xFF, 0xD9]);
        out
    }

    /// Where `bytes`' scan header ends (the walk, by hand: the frame and scan headers are the last two segments).
    fn header_len(bytes: &[u8], entropy: usize) -> usize {
        bytes.len() - entropy - 2
    }

    #[test]
    fn a_jpeg_header_ends_with_its_scan_header() {
        let file = jpeg_of(&[(0xE0, 14), (0xE1, 5000)], 100_000);
        let end = header_len(&file, 100_000);
        assert_eq!(jpeg_header_end(&file), JpegHeader::Ends(end));
        assert_eq!(jpeg_header_end(&file[..end]), JpegHeader::Ends(end), "the header alone is all of it");
        assert_eq!(jpeg_header_end(&file[..end - 1]), JpegHeader::Needs(end));
        // The start of image, the APP0 segment (18 bytes), the APP1 segment (5004), and the next marker's 4 bytes.
        assert_eq!(jpeg_header_end(&file[..100]), JpegHeader::Needs(2 + 18 + 5004 + 4));
        assert_eq!(jpeg_header_end(&file[..2]), JpegHeader::Needs(3));
        // Fill bytes before a marker are passed over.
        let mut filled = file.clone();
        filled.splice(2..2, [0xFF, 0xFF, 0xFF]);
        assert_eq!(jpeg_header_end(&filled), JpegHeader::Ends(end + 3));
    }

    #[test]
    fn a_jpeg_header_that_does_not_end_in_a_scan_is_unread() {
        assert_eq!(jpeg_header_end(&PNG_SIGNATURE), JpegHeader::Unread);
        // A marker byte that isn't 0xFF; a length under 2; the end marker before a scan.
        assert_eq!(jpeg_header_end(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 1, 2, 0x00, 0xDA, 0, 8]), JpegHeader::Unread);
        assert_eq!(jpeg_header_end(&[0xFF, 0xD8, 0xFF, 0xE1, 0, 1, 0, 0]), JpegHeader::Unread);
        assert_eq!(jpeg_header_end(&[0xFF, 0xD8, 0xFF, 0xD9, 0, 0]), JpegHeader::Unread);
        // No frame header before the scan; a scan header under 8 bytes.
        let no_frame = [0xFF, 0xD8, 0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0];
        assert_eq!(jpeg_header_end(&no_frame), JpegHeader::Unread);
        let mut short_scan = jpeg_of(&[], 10);
        let sos = short_scan.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        short_scan.splice(sos + 2..sos + 4, [0, 6]);
        assert_eq!(jpeg_header_end(&short_scan), JpegHeader::Unread);
    }

    fn jpeg_gather(bytes: &[u8]) -> Option<Vec<u8>> {
        jpeg_import_bytes(&mut std::io::Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn a_jpeg_is_read_to_the_end_of_its_header() {
        // A small header (one block); a long metadata segment past the first block; a header past 256 KiB.
        for segments in [vec![(0xE0, 14)], vec![(0xE1, 60_000), (0xE2, 3000)], vec![(0xE1, 65_000); 5]] {
            let file = jpeg_of(&segments, 500_000);
            let end = header_len(&file, 500_000);
            let read = jpeg_gather(&file).unwrap();
            assert!(read.len() >= end, "the whole header is read");
            assert!(read.len() < end + 2 * READ_BLOCK, "and no more than a block past it");
            assert_eq!(read, file[..read.len()], "the file's first bytes, as they are");
        }
        // A file shorter than a block is read whole.
        let small = jpeg_of(&[], 100);
        assert_eq!(jpeg_gather(&small), Some(small.clone()));
    }

    #[test]
    fn a_jpeg_whose_header_does_not_end_in_a_scan_is_read_whole() {
        assert_eq!(jpeg_gather(b"GIF89a\x01\x00\x01\x00"), None);
        assert_eq!(jpeg_gather(b""), None);
        // A header cut short: the scan header runs past the end of the file.
        let file = jpeg_of(&[(0xE1, 40_000)], 0);
        assert_eq!(jpeg_gather(&file[..30_000]), None);
        // The end marker before any scan.
        assert_eq!(jpeg_gather(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
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

    #[test]
    fn the_import_bytes_of_a_png_are_its_chunks_in_order_without_the_image_data() {
        let image = vec![7u8; 8000];
        let whole = png(&[
            (b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 3, 8, 2, 0, 0, 0]),
            (b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]),
            (b"eXIf", b"MM\0*"),
            (b"IDAT", image.as_slice()),
            (b"IDAT", image.as_slice()),
            (b"tEXt", b"Comment\0hello"),
            (b"fdAT", image.as_slice()),
            (b"eXIf", b"after"),
            (b"IEND", b""),
        ]);
        let gathered = png_import_bytes_of(&whole).unwrap();
        let kinds = |bytes: &[u8]| -> Vec<[u8; 4]> { png_chunks(bytes).map(|c| c.kind).collect() };
        // The image data is gone (one zero-length IDAT stands for it); every other chunk is there, in the file's order.
        let expected = [*b"IHDR", *b"acTL", *b"eXIf", *b"IDAT", *b"tEXt", *b"eXIf", *b"IEND"];
        assert_eq!(kinds(&gathered), expected);
        let first_exif = png_chunks(&gathered).find(|c| &c.kind == b"eXIf").unwrap();
        assert_eq!(first_exif.data, b"MM\0*");
        assert!(png_chunks(&gathered).find(|c| &c.kind == b"IDAT").unwrap().data.is_empty());
    }

    /// Bytes that look like compressed data (no chunk headers in them, but for the odd chance).
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = pg_testrng::Rng::new(seed);
        (0..len).map(|_| rng.next_u64().to_le_bytes()[0]).collect()
    }

    /// A PNG laid out as an encoder writes one: chunks before the image data, the image data's chunks of these lengths,
    /// the chunks after it, and `IEND`.
    fn png_of(before: &[(&[u8; 4], usize)], image: &[usize], after: &[(&[u8; 4], usize)]) -> Vec<u8> {
        let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"IHDR", vec![0, 0, 1, 0, 0, 0, 1, 0, 8, 2, 0, 0, 0])];
        let mut seed = 1;
        let mut add = |kind, len| {
            seed += 1;
            chunks.push((kind, noise(len, seed)));
        };
        before.iter().for_each(|&(kind, len)| add(kind, len));
        image.iter().for_each(|&len| add(b"IDAT", len));
        after.iter().for_each(|&(kind, len)| add(kind, len));
        add(b"IEND", 0);
        let parts: Vec<(&[u8; 4], &[u8])> = chunks.iter().map(|(kind, data)| (*kind, data.as_slice())).collect();
        png(&parts)
    }

    fn gather(bytes: &[u8]) -> Option<Vec<u8>> {
        png_import_bytes(&mut std::io::Cursor::new(bytes)).unwrap()
    }

    /// The gather of `bytes`, which must be made and be what the whole file gives.
    fn gathered_as_the_whole(bytes: &[u8]) {
        let whole = png_import_bytes_of(bytes).expect("the whole file gathers");
        assert_eq!(gather(bytes), Some(whole));
    }

    #[test]
    fn a_png_is_gathered_as_the_whole_file_gives_it() {
        let text = [(b"tEXt", 40), (b"eXIf", 900)];
        // One chunk of image data; a few small ones (the tail holds them all).
        gathered_as_the_whole(&png_of(&[], &[300_000], &text));
        gathered_as_the_whole(&png_of(&text, &[3000, 3000, 1200], &text));
        // Chunks of one length, the last shorter; the first shorter (as some encoders write it); short ones at the end.
        gathered_as_the_whole(&png_of(&[(b"pHYs", 9)], &[[8192; 300].as_slice(), &[5000]].concat(), &text));
        gathered_as_the_whole(&png_of(&[], &[[65_243].as_slice(), &[65_524; 40], &[12_000]].concat(), &[]));
        gathered_as_the_whole(&png_of(&[], &[[4096; 500].as_slice(), &[3398, 6]].concat(), &text));
        // The last chunk full, or the chunks large (the last one's header is before the tail).
        gathered_as_the_whole(&png_of(&[], &[1 << 20; 4], &text));
        gathered_as_the_whole(&png_of(&[], &[[1 << 20; 4].as_slice(), &[300_000]].concat(), &[]));
        // A long chunk before the image data (past the head), and after it (past the tail).
        gathered_as_the_whole(&png_of(&[(b"iCCP", 600_000)], &[8192; 100], &[(b"iTXt", 200_000)]));
        // The grid starting again after a short chunk.
        gathered_as_the_whole(&png_of(
            &[],
            &[[16_384; 384].as_slice(), &[15_340], &[16_384; 382], &[10_577]].concat(),
            &text,
        ));
    }

    #[test]
    fn a_png_with_bytes_after_its_iend_is_gathered_to_that_iend() {
        let image = [[32_768; 60].as_slice(), &[15_755]].concat();
        let file = png_of(&[], &image, &[(b"tEXt", 30)]);
        // A second IEND; bytes that are not a chunk; another image appended (its chunks off this one's grid).
        let mut twice = file.clone();
        twice.extend(PNG_IEND);
        gathered_as_the_whole(&twice);
        let mut junk = file.clone();
        junk.extend(noise(700_000, 9));
        gathered_as_the_whole(&junk);
        let mut appended = file.clone();
        appended.extend(png_of(&[(b"eXIf", 100)], &[[30_000; 20].as_slice(), &[777]].concat(), &[(b"eXIf", 50)]));
        gathered_as_the_whole(&appended);
    }

    #[test]
    fn a_png_the_gather_cannot_read_by_its_ends_is_read_whole() {
        // Not a PNG, no image data, animated, image data of many lengths, cut short, or more than the cap.
        assert_eq!(gather(b"GIF89a\x01\x00\x01\x00"), None);
        assert_eq!(gather(b""), None);
        assert_eq!(gather(&png_of(&[], &[], &[])), None);
        assert_eq!(gather(&png_of(&[(b"acTL", 8)], &[8192; 10], &[])), None);
        assert_eq!(gather(&png_of(&[], &[120_000, 118_000, 123_000, 101_000, 99_000], &[])), None);
        let mut cut = png_of(&[], &[8192; 10], &[(b"tEXt", 5000)]);
        cut.truncate(cut.len() - 2000);
        assert_eq!(gather(&cut), None);
        let big = usize::try_from(GATHER_CAP).unwrap();
        assert_eq!(gather(&png_of(&[], &[10], &[(b"eXIf", big)])), None);
    }

    use proptest::prelude::prop;

    proptest::proptest! {
        // Whatever the layout, the gather is the whole file's bytes, or none (the file is then read whole); and a file
        // laid out as an encoder writes it is gathered.
        #[test]
        fn a_gather_is_the_whole_files_or_none(
            before in prop::collection::vec(0usize..40_000, 0..3),
            first in 1usize..70_000,
            len in 1usize..70_000,
            count in 0usize..12,
            ends in prop::collection::vec(1usize..70_000, 0..3),
            after in prop::collection::vec(0usize..40_000, 0..3),
            restart in prop::option::of((1usize..70_000, 1usize..6)),
            trailer in 0usize..4,
        ) {
            let before: Vec<(&[u8; 4], usize)> = before.into_iter().map(|n| (b"tEXt", n)).collect();
            let after: Vec<(&[u8; 4], usize)> = after.into_iter().map(|n| (b"eXIf", n)).collect();
            let shorts: Vec<usize> = ends.iter().map(|&n| n % len + 1).collect();
            let mut image = vec![first];
            image.extend(std::iter::repeat_n(len, count));
            if let Some((short, more)) = restart {
                image.push(short % len + 1);
                image.extend(std::iter::repeat_n(len, more));
            }
            image.extend(&shorts);
            let mut file = png_of(&before, &image, &after);
            match trailer {
                1 => file.extend(PNG_IEND),
                2 => file.extend(noise(50_000, 3)),
                3 => file.extend(noise(30_000, 4).into_iter().chain(PNG_IEND)),
                _ => {}
            }
            let whole = png_import_bytes_of(&file);
            let gathered = gather(&file);
            proptest::prop_assert!(gathered.is_none() || gathered == whole);
            // One length after the first, the last ones shorter (the first short one at most a chunk), no restart.
            let regular = count >= 1 && restart.is_none() && shorts.len() <= 2 && shorts.iter().sum::<usize>() <= len;
            if regular && first <= len {
                proptest::prop_assert!(gathered.is_some(), "a regular layout is gathered");
            }
        }
    }
}
