//! Ratings, dates, orientation and frame counts as the files themselves hold them (PLAN §2: the catalog is the
//! source of truth; the file is read at import, [`Meta::read`], and written back by `pg-writeback`).
//!
//! The rating is read from the file's XMP packet, found where each container keeps it and parsed as XML with
//! namespaces resolved. A byte search could pick up the XMP of an embedded thumbnail or an edit history; this
//! only reads the packet the container declares as the image's own. EXIF Rating/RatingPercent (what Windows
//! writes) are the fallback.
//!
//! EXIF is read in place by a reader of its own: IFD0 and the Exif IFD, only the tags used here, every offset
//! checked. A broken field elsewhere in the block (a thumbnail's IFD, GPS) doesn't lose the ones read.
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::ops::Range;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::ReaderBuffers;

use pg_container::{
    ExtendedChunk, GifBlock, add_extended_chunk, chunk_data, find, gif_blocks, jpeg_exif, jpeg_segments, png_chunks,
    range_in, webp_chunks, webp_exif,
};
use pg_model::date::{exif_date, parse_iso_date};
use pg_model::format::Format;
use pg_model::orientation::Orientation;

const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_MICROSOFT_PHOTO: &str = "http://ns.microsoft.com/photo/1.0/";
const NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";
const NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const EXIF_TAG_RATING: u16 = 0x4746;
const EXIF_TAG_RATING_PERCENT: u16 = 0x4749;
const EXIF_TAG_ORIENTATION: u16 = 0x0112;
const TIFF_TAG_XMP: u16 = 700;

/// What the import reads from a file's metadata: one walk of its container, one read of its EXIF block, and one
/// parse of each XMP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    /// EXIF orientation 1–8; 1 when absent or unreadable.
    pub orientation: Orientation,
    /// The file's own rating: 0–5 (0 = explicitly unrated), or `None` when it holds none. XMP `xmp:Rating` first
    /// (the standard packet, then the extended one), then `MicrosoftPhoto:Rating`, then EXIF Rating and
    /// RatingPercent. XMP's -1 ("rejected") has no equivalent here and reads as no rating.
    pub rating: Option<u8>,
    /// When the picture was taken, as wall-clock time (no time zone: what the camera's clock showed) in Unix
    /// seconds: EXIF DateTimeOriginal, then DateTimeDigitized, then XMP `exif:DateTimeOriginal`,
    /// `photoshop:DateCreated` and `xmp:CreateDate`. `None` when the file says nothing.
    pub taken: Option<i64>,
    /// How many frames the file holds: more than one is an animated GIF, PNG (APNG) or WebP. Read from the
    /// container only (no decoding).
    pub frames: u32,
}

/// The XMP properties read from each packet: `xmp:Rating`, `MicrosoftPhoto:Rating`, and the dates.
const XMP_WANTED: [(&str, &str); 5] = [
    (NS_XMP, "Rating"),
    (NS_MICROSOFT_PHOTO, "Rating"),
    (NS_EXIF, "DateTimeOriginal"),
    (NS_PHOTOSHOP, "DateCreated"),
    (NS_XMP, "CreateDate"),
];

/// What reading a file's metadata works in besides the file, kept from one file to the next (a worker keeps one,
/// `analyze::Analyzer`): once its buffers have grown, reading a file's metadata allocates nothing.
#[derive(Default)]
pub struct MetaScratch {
    /// The container's XMP packets, main one first ([`Container::walk`]).
    packets: Vec<Packet>,
    /// The packets that aren't in the file as they are: reassembled extended JPEG packets, inflated PNG ones.
    assembled: Vec<u8>,
    /// An extended JPEG packet's chunks, as found.
    chunks: Vec<ExtendedChunk>,
    /// The inflater of compressed PNG packets, made when one is first met.
    inflater: Option<Box<DecompressorOxide>>,
    /// The XML reader's.
    xml: ReaderBuffers,
    /// The wanted values of the packet being read, one after another.
    values: String,
}

/// An XMP packet: a range of the file, or of [`MetaScratch::assembled`].
#[derive(Clone)]
enum Packet {
    InFile(Range<usize>),
    Assembled(Range<usize>),
}

impl Meta {
    pub fn read(bytes: &[u8], format: Format, scratch: &mut MetaScratch) -> Meta {
        let container = Container::walk(bytes, format, scratch);
        let exif = container
            .exif
            .clone()
            .and_then(|block| Exif::read(bytes.get(block)?, format == Format::Tiff))
            .unwrap_or_default();
        if format == Format::Tiff {
            // A TIFF file keeps its packet in IFD0.
            scratch.packets.extend(exif.xmp.map(|x| Packet::InFile(range_in(bytes, x))));
        }
        // In packet order: the first packet's value of each, but MicrosoftPhoto's only counts without an XMP one.
        let (mut xmp_rating, mut microsoft, mut xmp_taken) = (None, None, None);
        let MetaScratch { packets, assembled, xml, values, .. } = scratch;
        for packet in packets.iter() {
            let packet = match packet {
                Packet::InFile(r) => bytes.get(r.clone()),
                Packet::Assembled(r) => assembled.get(r.clone()),
            };
            let Some(packet) = packet else { continue };
            let found = xmp_properties(packet, XMP_WANTED, xml, values);
            #[expect(
                clippy::string_slice,
                reason = "each range is a whole string pushed into `values`: a char boundary"
            )]
            let [rating, ms, original, created, create] = found.map(|v| v.map(|r| &values[r]));
            let number = |v: Option<&str>| v.and_then(|v| v.parse::<f64>().ok());
            xmp_rating = xmp_rating.or_else(|| number(rating).and_then(xmp_value));
            microsoft = microsoft.or_else(|| number(ms).map(percent));
            xmp_taken =
                xmp_taken.or_else(|| [original, created, create].into_iter().flatten().find_map(parse_iso_date));
        }
        let exif_rating = || {
            exif.rating
                .map(|v| u8::try_from(v.min(5)).unwrap_or(u8::MAX))
                .or_else(|| exif.rating_percent.map(|v| percent(f64::from(v))))
        };
        Meta {
            orientation: exif.orientation(),
            rating: xmp_rating.or(microsoft).or_else(exif_rating),
            taken: exif.dates.iter().flatten().find_map(|d| exif_date(d)).or(xmp_taken),
            frames: container.frames,
        }
    }
}

/// The EXIF orientation (1–8) of a file, alone; 1 when absent or unreadable.
pub fn orientation(bytes: &[u8], format: Format) -> Orientation {
    exif_block(bytes, format)
        .and_then(|block| Exif::read(bytes.get(block)?, format == Format::Tiff))
        .unwrap_or_default()
        .orientation()
}

/// The first value of each wanted property (namespace URI, local name) in one packet, as an attribute of an
/// `rdf:Description` or as an element's text, trimmed; empty values are skipped. Each is a range of `values`
/// (emptied first), read with the XML reader in `buffers` (given back after).
fn xmp_properties<const N: usize>(
    packet: &[u8],
    wanted: [(&str, &str); N],
    buffers: &mut ReaderBuffers,
    values: &mut String,
) -> [Option<Range<usize>>; N] {
    values.clear();
    let text = String::from_utf8_lossy(packet);
    // The packet wrapper (<?xpacket ...?>) is a processing instruction, which the parser skips.
    let mut reader = NsReader::from_str_with(&text, std::mem::take(buffers));
    let mut found: [Option<Range<usize>>; N] = std::array::from_fn(|_| None);
    // Set while inside a wanted element, to read its text.
    let mut inside: Option<usize> = None;
    let which = |ns: &ResolveResult, local: &str| {
        wanted
            .iter()
            .position(|(uri, name)| matches!(ns, ResolveResult::Bound(Namespace(n)) if n == uri) && local == *name)
    };
    let mut keep = |found: &mut [Option<Range<usize>>; N], i: usize, value: &str| {
        let value = value.trim();
        if let Some(slot) = found.get_mut(i)
            && slot.is_none()
            && !value.is_empty()
        {
            let start = values.len();
            values.push_str(value);
            *slot = Some(start..values.len());
        }
    };
    while let Ok((ns, event)) = reader.read_resolved_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                inside = which(&ns, e.local_name().as_ref());
                for attr in e.attributes().flatten() {
                    let (ans, local) = reader.resolver().resolve_attribute(attr.key);
                    if let Some(i) = which(&ans, local.as_ref())
                        && let Ok(v) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    {
                        keep(&mut found, i, &v);
                    }
                }
            }
            Event::Text(t) => {
                if let Some(i) = inside
                    && let Ok(v) = quick_xml::escape::unescape(&t)
                {
                    keep(&mut found, i, &v);
                }
            }
            Event::End(_) => inside = None,
            Event::Eof => break,
            _ => {}
        }
    }
    *buffers = reader.into_buffers();
    found
}

/// XMP rating: a real number in -1..=5; -1 (rejected) and anything out of range read as no rating.
fn xmp_value(v: f64) -> Option<u8> {
    (0.0..=5.0).contains(&v).then(|| round_u8(v))
}

/// Windows' percentage scale (1, 25, 50, 75, 99), as ImageRater mapped it.
fn percent(v: f64) -> u8 {
    match round_u8(v) {
        ..=0 => 0,
        1..=24 => 1,
        25..=49 => 2,
        50..=74 => 3,
        75..=98 => 4,
        _ => 5,
    }
}

/// `v` rounded to the nearest whole number, held to 0..=255 (NaN reads as 0).
fn round_u8(v: f64) -> u8 {
    pg_model::num::f64_to_u8(v.round().clamp(0.0, 255.0))
}

// --- EXIF -------------------------------------------------------------------------------------------------------

/// A TIFF structure (an EXIF block, or a TIFF file), read in place: every offset is checked against it.
struct Tiff<'a> {
    data: &'a [u8],
    little_endian: bool,
    /// BigTIFF: 8-byte counts and offsets, 20-byte entries, an 8-byte value field.
    big: bool,
}

/// An IFD entry: its tag, type, count, and where it is (its 12 bytes, or 20 in BigTIFF).
struct Entry {
    tag: u16,
    kind: u16,
    count: u64,
    at: usize,
}

const TIFF_BYTE: u16 = 1;
const TIFF_ASCII: u16 = 2;
const TIFF_SHORT: u16 = 3;
const TIFF_LONG: u16 = 4;
const TIFF_UNDEFINED: u16 = 7;
const TIFF_LONG8: u16 = 16;
const TIFF_IFD8: u16 = 18;

impl<'a> Tiff<'a> {
    /// The byte order, then 42. A TIFF file (`bigtiff`) may be BigTIFF instead: 43, then 8 (the offset size) and 0.
    /// An EXIF block is always classic.
    fn new(data: &'a [u8], bigtiff: bool) -> Option<Tiff<'a>> {
        let little_endian = match data.get(..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let mut tiff = Tiff { data, little_endian, big: false };
        tiff.big = match tiff.u16_at(2)? {
            42 => false,
            43 if bigtiff && tiff.u16_at(4)? == 8 && tiff.u16_at(6)? == 0 => true,
            _ => return None,
        };
        Some(tiff)
    }

    fn u16_at(&self, i: usize) -> Option<u16> {
        self.u16_of(self.data.get(i..i.checked_add(2)?)?)
    }

    fn u32_at(&self, i: usize) -> Option<u32> {
        self.u32_of(self.data.get(i..i.checked_add(4)?)?)
    }

    /// The SHORT at the start of `b`, in the block's byte order.
    fn u16_of(&self, b: &[u8]) -> Option<u16> {
        let b: [u8; 2] = b.get(..2)?.try_into().ok()?;
        Some(if self.little_endian { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    /// The LONG at the start of `b`, in the block's byte order.
    fn u32_of(&self, b: &[u8]) -> Option<u32> {
        let b: [u8; 4] = b.get(..4)?.try_into().ok()?;
        Some(if self.little_endian { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The LONG8 at `i` (BigTIFF), in the block's byte order.
    fn u64_at(&self, i: usize) -> Option<u64> {
        self.u64_of(self.data.get(i..i.checked_add(8)?)?)
    }

    /// The LONG8 at the start of `b`, in the block's byte order.
    fn u64_of(&self, b: &[u8]) -> Option<u64> {
        let b: [u8; 8] = b.get(..8)?.try_into().ok()?;
        Some(if self.little_endian { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    }

    /// An offset at `i`: a LONG, or a LONG8 in BigTIFF.
    fn offset_at(&self, i: usize) -> Option<usize> {
        if self.big { usize::try_from(self.u64_at(i)?).ok() } else { Some(pg_model::num::index(self.u32_at(i)?)) }
    }

    /// IFD0's offset.
    fn ifd0(&self) -> Option<usize> {
        self.offset_at(if self.big { 8 } else { 4 })
    }

    /// The entries of the IFD at `offset`, as far as they are in the block.
    fn entries(&self, offset: usize) -> impl Iterator<Item = Entry> + '_ {
        let (head, size): (usize, usize) = if self.big { (8, 20) } else { (2, 12) };
        let count = if self.big { self.u64_at(offset) } else { self.u16_at(offset).map(u64::from) };
        let count = usize::try_from(count.unwrap_or(0)).unwrap_or(usize::MAX);
        (0..count).map_while(move |k| {
            let at = offset.checked_add(head)?.checked_add(size.checked_mul(k)?)?;
            let count = if self.big { self.u64_at(at + 4)? } else { u64::from(self.u32_at(at + 4)?) };
            Some(Entry { tag: self.u16_at(at)?, kind: self.u16_at(at + 2)?, count, at })
        })
    }

    /// Where an entry's value field starts: 8 bytes into a classic entry, 12 into a BigTIFF one.
    fn value_at(&self, e: &Entry) -> usize {
        e.at + if self.big { 12 } else { 8 }
    }

    /// An entry's value bytes: in the entry when they fit in 4 bytes, else where it points. `None` for a type
    /// whose size isn't known, or a value outside the block.
    fn value(&self, e: &Entry) -> Option<&'a [u8]> {
        self.data.get(self.value_range(e)?)
    }

    /// Where an entry's value bytes are in the block: its value field when they fit there (4 bytes, or 8 in
    /// BigTIFF), else where that field points. `None` for a type whose size isn't known, or a value outside the
    /// block.
    fn value_range(&self, e: &Entry) -> Option<Range<usize>> {
        let unit: usize = match e.kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            // LONG8, SLONG8 and IFD8: BigTIFF only.
            16..=18 if self.big => 8,
            _ => return None,
        };
        let len = unit.checked_mul(usize::try_from(e.count).ok()?)?;
        let inline = if self.big { 8 } else { 4 };
        let at = self.value_at(e);
        let start = if len <= inline { at } else { self.offset_at(at)? };
        let range = start..start.checked_add(len)?;
        self.data.get(range.clone())?;
        Some(range)
    }

    /// Whether an orientation entry is of a kind the reader and the patcher take: BYTE, SHORT or LONG, and LONG8
    /// in BigTIFF (the kinds [`Tiff::uint`] reads as a number).
    fn orientation_kind(&self, e: &Entry) -> bool {
        matches!(e.kind, TIFF_BYTE | TIFF_SHORT | TIFF_LONG) || (self.big && e.kind == TIFF_LONG8)
    }

    /// IFD0's orientation entries of a kind [`Tiff::orientation_kind`] takes, each with where its value bytes are
    /// in the block (an entry whose value is outside the block is left out). The reader takes the last readable
    /// one; the patcher sets the first value of each.
    fn orientation_entries(&self) -> impl Iterator<Item = (Range<usize>, Entry)> + '_ {
        self.ifd0()
            .into_iter()
            .flat_map(|ifd0| self.entries(ifd0))
            .filter(|e| e.tag == EXIF_TAG_ORIENTATION && self.orientation_kind(e))
            .filter_map(|e| Some((self.value_range(&e)?, e)))
    }

    /// An entry's first value as an unsigned integer (BYTE, SHORT or LONG; a LONG8 or IFD8 that fits a LONG).
    fn uint(&self, e: &Entry) -> Option<u32> {
        let v = self.value(e)?;
        match e.kind {
            TIFF_BYTE => v.first().map(|&b| u32::from(b)),
            TIFF_SHORT => self.u16_of(v).map(u32::from),
            TIFF_LONG => self.u32_of(v),
            TIFF_LONG8 | TIFF_IFD8 => self.u64_of(v).and_then(|n| u32::try_from(n).ok()),
            _ => None,
        }
    }

    /// An ASCII entry's first string (up to its first NUL).
    fn ascii(&self, e: &Entry) -> Option<&'a [u8]> {
        let v = self.value(e).filter(|_| e.kind == TIFF_ASCII)?;
        v.split(|&b| b == 0).next()
    }

    /// A BYTE or UNDEFINED entry's bytes.
    fn bytes(&self, e: &Entry) -> Option<&'a [u8]> {
        self.value(e).filter(|_| matches!(e.kind, TIFF_BYTE | TIFF_UNDEFINED))
    }
}

/// What the import reads of an EXIF block: IFD0's orientation, rating and XMP (where a TIFF file keeps its
/// packet), and the Exif IFD's dates. A field that doesn't fit in the block reads as absent, and the rest are still
/// read; a tag given twice reads as the last one.
#[derive(Default)]
struct Exif<'a> {
    orientation: Option<u32>,
    rating: Option<u32>,
    rating_percent: Option<u32>,
    xmp: Option<&'a [u8]>,
    /// DateTimeOriginal, then DateTimeDigitized.
    dates: [Option<&'a [u8]>; 2],
}

const EXIF_TAG_EXIF_IFD: u16 = 0x8769;
const EXIF_TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const EXIF_TAG_DATE_TIME_DIGITIZED: u16 = 0x9004;

impl<'a> Exif<'a> {
    fn read(block: &'a [u8], bigtiff: bool) -> Option<Exif<'a>> {
        let tiff = Tiff::new(block, bigtiff)?;
        let mut exif = Exif::default();
        let mut exif_ifd = None;
        for e in tiff.entries(tiff.ifd0()?) {
            match e.tag {
                // The last readable one, of the kinds the patcher sets too.
                EXIF_TAG_ORIENTATION if tiff.orientation_kind(&e) => {
                    exif.orientation = tiff.uint(&e).or(exif.orientation);
                }
                EXIF_TAG_RATING => exif.rating = tiff.uint(&e),
                EXIF_TAG_RATING_PERCENT => exif.rating_percent = tiff.uint(&e),
                TIFF_TAG_XMP => exif.xmp = tiff.bytes(&e),
                EXIF_TAG_EXIF_IFD => exif_ifd = tiff.uint(&e),
                _ => {}
            }
        }
        for e in exif_ifd.into_iter().flat_map(|ifd| tiff.entries(pg_model::num::index(ifd))) {
            match e.tag {
                EXIF_TAG_DATE_TIME_ORIGINAL => exif.dates[0] = tiff.ascii(&e),
                EXIF_TAG_DATE_TIME_DIGITIZED => exif.dates[1] = tiff.ascii(&e),
                _ => {}
            }
        }
        Some(exif)
    }

    fn orientation(&self) -> Orientation {
        self.orientation.and_then(|o| u8::try_from(o).ok()).map_or(Orientation::UPRIGHT, Orientation::from_tag)
    }
}

// --- Containers -------------------------------------------------------------------------------------------------

/// Where a file's container keeps the metadata the import reads, found in one walk of it. Its XMP packets, main one
/// first, are in [`MetaScratch::packets`]: in the file, but for a reassembled extended JPEG packet and a compressed
/// PNG one, in [`MetaScratch::assembled`]. (A TIFF file's is in its IFD0, read with the EXIF block.)
struct Container {
    /// The EXIF block (a TIFF structure).
    exif: Option<Range<usize>>,
    frames: u32,
}

impl Container {
    fn walk(bytes: &[u8], format: Format, scratch: &mut MetaScratch) -> Container {
        let mut c = Container { exif: None, frames: 1 };
        let MetaScratch { packets, assembled, chunks, inflater, .. } = scratch;
        packets.clear();
        assembled.clear();
        match format {
            Format::Jpeg => {
                chunks.clear();
                for s in jpeg_segments(bytes).filter(|s| s.marker == 0xE1) {
                    if c.exif.is_none() {
                        c.exif = jpeg_exif(&s);
                    }
                    if let Some(packet) = s.data.strip_prefix(JPEG_XMP) {
                        packets.push(Packet::InFile(range_in(bytes, packet)));
                    } else {
                        add_extended_chunk(bytes, s.data, chunks);
                    }
                }
                // The last standard packet first, then the one before it.
                packets.reverse();
                extended_packets(bytes, chunks, assembled, packets);
            }
            Format::Png => {
                // acTL counts before the first IDAT.
                let (mut image_data, mut animation) = (false, None);
                for chunk in png_chunks(bytes) {
                    match &chunk.kind {
                        b"eXIf" if c.exif.is_none() => c.exif = Some(chunk_data(&chunk)),
                        b"iTXt" => match png_itxt_xmp(chunk.data) {
                            Some(ItxtXmp::Plain(text)) => packets.push(Packet::InFile(range_in(bytes, text))),
                            Some(ItxtXmp::Compressed(text)) => {
                                let inflater = inflater.get_or_insert_with(Box::default);
                                // The budget is for the file: what its earlier packets left of it.
                                let left = MAX_INFLATED.saturating_sub(assembled.len());
                                packets.extend(inflate_zlib(text, inflater, assembled, left).map(Packet::Assembled));
                            }
                            None => {}
                        },
                        b"IDAT" => image_data = true,
                        b"acTL" if !image_data && animation.is_none() => {
                            animation = Some(chunk.data.first_chunk::<4>().map(|&n| u32::from_be_bytes(n)));
                        }
                        _ => {}
                    }
                }
                c.frames = animation.flatten().unwrap_or(1).max(1);
            }
            Format::Webp => {
                let (mut animated, mut frames) = (false, 0);
                for chunk in webp_chunks(bytes) {
                    match &chunk.kind {
                        b"VP8X" => animated |= chunk.data.first().is_some_and(|f| f & 0x02 != 0),
                        b"ANMF" => frames += 1,
                        b"EXIF" if c.exif.is_none() => c.exif = Some(webp_exif(&chunk)),
                        b"XMP " => packets.push(Packet::InFile(chunk_data(&chunk))),
                        _ => {}
                    }
                }
                c.frames = if animated { frames.max(1) } else { 1 };
            }
            Format::Gif => {
                let mut blocks = gif_blocks(bytes);
                let (mut images, mut packet) = (0, None);
                for b in blocks.by_ref() {
                    match b {
                        GifBlock::Image { .. } => images += 1,
                        GifBlock::Extension { label: 0xFF, range } if packet.is_none() => {
                            packet = gif_xmp(bytes, range);
                        }
                        GifBlock::Extension { .. } => {}
                    }
                }
                // Only a GIF read to its trailer.
                if blocks.complete() {
                    c.frames = images.max(1);
                    packets.extend(packet.map(|x| Packet::InFile(range_in(bytes, x))));
                }
            }
            Format::Tiff => c.exif = Some(0..bytes.len()),
            Format::Bmp => {}
        }
        c
    }
}

/// The extended packets whose chunks are all there, by GUID, each reassembled into `assembled`. A packet's length
/// is what its first chunk found says; of chunks at the same offset, the last found counts; the chunks must follow
/// each other from offset 0 up to that length.
fn extended_packets(bytes: &[u8], chunks: &mut [ExtendedChunk], assembled: &mut Vec<u8>, packets: &mut Vec<Packet>) {
    chunks.sort_unstable_by(|a, b| {
        (bytes.get(a.guid.clone()), a.offset, a.seq).cmp(&(bytes.get(b.guid.clone()), b.offset, b.seq))
    });
    for packet in chunks.chunk_by(|a, b| bytes.get(a.guid.clone()) == bytes.get(b.guid.clone())) {
        let Some(full) = packet.iter().min_by_key(|c| c.seq).map(|c| c.full) else { continue };
        let start = assembled.len();
        for last in packet.chunk_by(|a, b| a.offset == b.offset).filter_map(<[_]>::last) {
            if last.offset != assembled.len() - start {
                break;
            }
            let Some(data) = bytes.get(last.data.clone()) else { break };
            assembled.extend_from_slice(data);
        }
        if assembled.len() - start == full {
            packets.push(Packet::Assembled(start..assembled.len()));
        } else {
            assembled.truncate(start);
        }
    }
}

/// The most the inflated XMP packets of one file may be, together. A packet is a few kilobytes; a larger one (a
/// compressed bomb in a small file), or one past what the file's earlier packets left, is treated as absent, so the
/// import doesn't grow to gigabytes for it, however many compressed chunks the file holds.
const MAX_INFLATED: usize = 16 << 20;

/// zlib data inflated onto the end of `out`, as `miniz_oxide::inflate::decompress_to_vec_zlib` would give it (the
/// same output, the same failures), with a kept decompressor. Its range in `out`; `None`, and `out` as it was, if it
/// doesn't inflate or inflates to more than `limit` bytes (the budget left of [`MAX_INFLATED`]; none left, it is
/// absent).
fn inflate_zlib(
    mut input: &[u8],
    decompressor: &mut DecompressorOxide,
    out: &mut Vec<u8>,
    limit: usize,
) -> Option<Range<usize>> {
    if limit == 0 {
        return None;
    }
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    decompressor.init();
    let start = out.len();
    out.resize(start + input.len().saturating_mul(2).min(limit), 0);
    let mut at = 0;
    loop {
        let (status, read, written) = decompress(decompressor, input, out.get_mut(start..)?, at, flags);
        at += written;
        // The input left: `None` if the decompressor says it read more than it was given.
        match (status, input.get(read..)) {
            (TINFLStatus::Done, _) => {
                out.truncate(start + at);
                return Some(start..out.len());
            }
            (TINFLStatus::HasMoreOutput, Some(rest)) if out.len() - start < limit => {
                input = rest;
                let len = out.len() - start;
                out.resize(start + len.saturating_mul(2).min(limit), 0);
            }
            _ => {
                out.truncate(start);
                return None;
            }
        }
    }
}

/// The text of an `XML:com.adobe.xmp` iTXt chunk.
enum ItxtXmp<'a> {
    Plain(&'a [u8]),
    /// zlib data.
    Compressed(&'a [u8]),
}

fn png_itxt_xmp(data: &[u8]) -> Option<ItxtXmp<'_>> {
    let rest = data.strip_prefix(b"XML:com.adobe.xmp\0")?;
    let (&compressed, rest) = rest.split_first()?;
    let rest = rest.get(1..)?; // compression method
    let lang_end = rest.iter().position(|&b| b == 0)?;
    let rest = rest.get(lang_end + 1..)?;
    let translated_end = rest.iter().position(|&b| b == 0)?;
    let text = rest.get(translated_end + 1..)?;
    Some(if compressed == 1 { ItxtXmp::Compressed(text) } else { ItxtXmp::Plain(text) })
}

/// The XMP packet of an application extension (`range`: the whole block), if it holds one.
fn gif_xmp(bytes: &[u8], range: std::ops::Range<usize>) -> Option<&[u8]> {
    if bytes.get(range.start + 3..range.start + 14) != Some(b"XMP DataXMP") {
        return None;
    }
    let data = bytes.get(range.start + 14..range.end)?;
    let end = find(data, b"<?xpacket end=")?;
    let close = find(data.get(end..)?, b"?>")?;
    data.get(..end + close + 2)
}

/// Where the EXIF block (a TIFF structure) of a file is, if it has one.
fn exif_block(bytes: &[u8], format: Format) -> Option<std::ops::Range<usize>> {
    match format {
        Format::Jpeg => jpeg_segments(bytes).find_map(|s| jpeg_exif(&s)),
        Format::Png => png_chunks(bytes).find(|c| &c.kind == b"eXIf").map(|c| chunk_data(&c)),
        Format::Webp => webp_chunks(bytes).find(|c| &c.kind == b"EXIF").map(|c| webp_exif(&c)),
        Format::Tiff => Some(0..bytes.len()),
        Format::Gif | Format::Bmp => None,
    }
}

/// The value bytes to write for each orientation entry in a TIFF block's IFD0 (those [`Tiff::orientation_entries`]
/// yields): the range of the entry's first value in the block, and `o` at that entry's width (BYTE, SHORT, LONG or
/// LONG8), in the block's byte order.
fn tiff_orientation_patches(block: &[u8], bigtiff: bool, o: Orientation) -> Option<Vec<(Range<usize>, Vec<u8>)>> {
    let tiff = Tiff::new(block, bigtiff)?;
    let le = tiff.little_endian;
    let v = o.get();
    let patches = tiff
        .orientation_entries()
        .filter_map(|(range, e)| {
            let value: Vec<u8> = match e.kind {
                TIFF_BYTE => vec![v],
                TIFF_SHORT => (if le { u16::from(v).to_le_bytes() } else { u16::from(v).to_be_bytes() }).to_vec(),
                TIFF_LONG => (if le { u32::from(v).to_le_bytes() } else { u32::from(v).to_be_bytes() }).to_vec(),
                TIFF_LONG8 => (if le { u64::from(v).to_le_bytes() } else { u64::from(v).to_be_bytes() }).to_vec(),
                _ => return None,
            };
            let first = range.start..range.start.checked_add(value.len())?;
            // The entry's value holds at least one value of its width (its count is not 0).
            (first.end <= range.end).then_some((first, value))
        })
        .collect();
    Some(patches)
}

/// The file with its EXIF orientation set to `o`, changed in place (same size, nothing moves), or `None` when
/// the file has no orientation tag to change (adding one is a job for ExifTool). Every IFD0 orientation entry the
/// reader could take is set, each at its own width, so the reader sees `o` whichever entry it takes.
pub fn patch_orientation(bytes: &[u8], format: Format, o: Orientation) -> Option<Vec<u8>> {
    let block = exif_block(bytes, format)?;
    let patches = tiff_orientation_patches(bytes.get(block.clone())?, format == Format::Tiff, o)?;
    if patches.is_empty() {
        return None;
    }
    let mut out = bytes.to_vec();
    for (range, value) in patches {
        let at = block.start.checked_add(range.start)?..block.start.checked_add(range.end)?;
        out.get_mut(at)?.copy_from_slice(&value);
    }
    if format == Format::Png {
        // The eXIf chunk's CRC covers its type and data.
        let chunk = png_chunks(&out).find(|c| &c.kind == b"eXIf")?;
        let (start, end) = (chunk.at + 4, chunk.at + 8 + chunk.data.len());
        let crc = crc32fast::hash(out.get(start..end)?);
        out.get_mut(end..end + 4)?.copy_from_slice(&crc.to_be_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_container::{JPEG_XMP_EXTENSION, gif_header_len};
    use pg_model::date::wall_clock_seconds;
    use pg_testkit::{apng, gif};

    fn meta_of(bytes: &[u8], format: Format) -> Meta {
        Meta::read(bytes, format, &mut MetaScratch::default())
    }

    fn rating(bytes: &[u8], format: Format) -> Option<u8> {
        meta_of(bytes, format).rating
    }

    fn taken(bytes: &[u8], format: Format) -> Option<i64> {
        meta_of(bytes, format).taken
    }

    fn frames(bytes: &[u8], format: Format) -> u32 {
        meta_of(bytes, format).frames
    }

    /// A WebP container holding only these chunks (enough for the metadata read, which decodes nothing).
    fn webp(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        for (kind, data) in chunks {
            body.extend([&kind[..], &u32::try_from(data.len()).unwrap().to_le_bytes(), data].concat());
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        [&b"RIFF"[..], &u32::try_from(body.len()).unwrap().to_le_bytes(), &body].concat()
    }

    #[test]
    fn xmp_attribute_element_and_prefixes() {
        let attr =
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4"/></rdf:RDF></x:xmpmeta>"#;
        let xmp = |packet: &[u8]| rating(&webp(&[(b"XMP ", packet)]), Format::Webp);
        assert_eq!(xmp(attr), Some(4));
        // Any prefix bound to the XMP namespace counts; a different namespace with the same prefix doesn't.
        let element = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:a="http://ns.adobe.com/xap/1.0/" xmlns:xmp="urn:other"><xmp:Rating>2</xmp:Rating>
            <a:Rating>5</a:Rating></rdf:Description></rdf:RDF>"#;
        assert_eq!(xmp(element), Some(5));
        let windows = br#"<rdf:Description xmlns:MicrosoftPhoto="http://ns.microsoft.com/photo/1.0/"
            MicrosoftPhoto:Rating="75"/>"#;
        assert_eq!(xmp(windows), Some(4));
        let rejected = br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="-1"/>"#;
        assert_eq!(xmp(rejected), None);
        // An XMP rating in a later packet wins over a MicrosoftPhoto one in the first.
        assert_eq!(rating(&webp(&[(b"XMP ", windows), (b"XMP ", element)]), Format::Webp), Some(5));
    }

    #[test]
    fn percent_scale() {
        assert_eq!([0.0, 1.0, 25.0, 50.0, 75.0, 99.0, 100.0].map(percent), [0, 1, 2, 3, 4, 5, 5]);
    }

    /// A little-endian TIFF block whose Exif IFD holds DateTimeOriginal.
    fn tiff_taken(date: &[u8; 19]) -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        // IFD0: one entry, the Exif IFD pointer (LONG) → 26.
        t.extend(1u16.to_le_bytes());
        t.extend([0x69, 0x87, 4, 0]);
        t.extend(1u32.to_le_bytes());
        t.extend(26u32.to_le_bytes());
        t.extend(0u32.to_le_bytes());
        // Exif IFD: DateTimeOriginal (ASCII, 20 bytes) at 44.
        t.extend(1u16.to_le_bytes());
        t.extend([0x03, 0x90, 2, 0]);
        t.extend(20u32.to_le_bytes());
        t.extend(44u32.to_le_bytes());
        t.extend(0u32.to_le_bytes());
        t.extend(date);
        t.push(0);
        t
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        c.extend(kind);
        c.extend(data);
        c.extend(crc32fast::hash(&[&kind[..], data].concat()).to_be_bytes());
        c
    }

    /// An IFD entry for [`tiff_block`]: tag, type, count, and the value's bytes (in the block's byte order).
    type TestEntry = (u16, u16, u32, Vec<u8>);

    /// A TIFF block with these IFD0 entries and, if any, an Exif IFD with those (its pointer added to IFD0). Values
    /// over the entry's value field go after the IFDs.
    fn tiff_block(le: bool, ifd0: &[TestEntry], exif: &[TestEntry]) -> Vec<u8> {
        tiff_block_as(false, le, ifd0, exif)
    }

    /// [`tiff_block`], or a BigTIFF block (`big`): 43, 8-byte counts and offsets, 20-byte entries, 8-byte value fields.
    fn tiff_block_as(big: bool, le: bool, ifd0: &[TestEntry], exif: &[TestEntry]) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u64b = |v: u64| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        // An offset, or a value's count, as the layout's width: 4 bytes, or 8 in BigTIFF.
        let width: usize = if big { 8 } else { 4 };
        let offset = |v: usize| {
            if big { u64b(u64::try_from(v).unwrap()).to_vec() } else { u32b(u32::try_from(v).unwrap()).to_vec() }
        };
        // The IFD's entry count is 2 bytes, or 8 in BigTIFF; an entry is 12, or 20.
        let (count_w, entry) = if big { (8, 20) } else { (2, 12) };
        let head = if big { 16 } else { 8 };
        let ifd_len = |n: usize| count_w + entry * n + width;
        let n0 = ifd0.len() + usize::from(!exif.is_empty());
        let exif_at = head + ifd_len(n0);
        let mut data_at = exif_at + if exif.is_empty() { 0 } else { ifd_len(exif.len()) };
        let mut data: Vec<u8> = Vec::new();
        let mut ifd = |entries: &[TestEntry], out: &mut Vec<u8>| {
            out.extend(if big {
                u64b(u64::try_from(entries.len()).unwrap()).to_vec()
            } else {
                u16b(u16::try_from(entries.len()).unwrap()).to_vec()
            });
            for (tag, kind, count, value) in entries {
                out.extend(u16b(*tag));
                out.extend(u16b(*kind));
                out.extend(if big { u64b(u64::from(*count)).to_vec() } else { u32b(*count).to_vec() });
                if value.len() <= width {
                    out.extend(value);
                    out.extend(std::iter::repeat_n(0, width - value.len()));
                } else {
                    out.extend(offset(data_at));
                    data.extend(value);
                    data_at += value.len();
                }
            }
            out.extend(offset(0));
        };
        let mut block = if le { b"II".to_vec() } else { b"MM".to_vec() };
        if big {
            block.extend(u16b(43));
            block.extend(u16b(8));
            block.extend(u16b(0));
            block.extend(u64b(16));
        } else {
            block.extend(u16b(42));
            block.extend(u32b(8));
        }
        let mut entries = ifd0.to_vec();
        if !exif.is_empty() {
            // The Exif IFD's pointer: an IFD8 in BigTIFF, a LONG otherwise.
            let pointer = if big {
                (TIFF_IFD8, u64b(u64::try_from(exif_at).unwrap()).to_vec())
            } else {
                (TIFF_LONG, u32b(u32::try_from(exif_at).unwrap()).to_vec())
            };
            entries.push((EXIF_TAG_EXIF_IFD, pointer.0, 1, pointer.1));
        }
        ifd(&entries, &mut block);
        if !exif.is_empty() {
            ifd(exif, &mut block);
        }
        block.extend(data);
        block
    }

    /// An offset in a TIFF block: a LONG, or a LONG8 in BigTIFF.
    fn offset_bytes(big: bool, le: bool, v: usize) -> Vec<u8> {
        match (big, le) {
            (false, true) => u32::try_from(v).unwrap().to_le_bytes().to_vec(),
            (false, false) => u32::try_from(v).unwrap().to_be_bytes().to_vec(),
            (true, true) => u64::try_from(v).unwrap().to_le_bytes().to_vec(),
            (true, false) => u64::try_from(v).unwrap().to_be_bytes().to_vec(),
        }
    }

    #[test]
    fn exif_reads_what_fits_and_the_last_of_a_tag() {
        for (big, le) in [(false, true), (false, false), (true, true), (true, false)] {
            let short =
                |v: &[u16]| v.iter().flat_map(|&v| if le { v.to_le_bytes() } else { v.to_be_bytes() }).collect();
            let long = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() }.to_vec();
            let date = |d: &[u8]| (EXIF_TAG_DATE_TIME_ORIGINAL, TIFF_ASCII, 20, [d, b"\0"].concat());
            let read =
                |ifd0: &[TestEntry], exif: &[TestEntry]| meta_of(&tiff_block_as(big, le, ifd0, exif), Format::Tiff);

            let m = read(&[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[6]))], &[date(b"2023:07:15 18:30:05")]);
            assert_eq!((m.orientation.get(), m.taken), (6, Some(1_689_445_805)));
            // A SHORT array stored out of the entry: its first value. BYTE and LONG count too; other types don't.
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 3, short(&[8, 1, 1]))], &[]).orientation.get(), 8);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![3])], &[]).orientation.get(), 3);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_LONG, 1, long(5))], &[]).orientation.get(), 5);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_ASCII, 2, b"6\0".to_vec())], &[]).orientation.get(), 1);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[9]))], &[]).orientation.get(), 1);
            // A tag given twice reads as the last.
            let twice = [
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[6])),
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[3])),
            ];
            assert_eq!(read(&twice, &[]).orientation.get(), 3);
            // Rating, capped at 5; RatingPercent only without it.
            assert_eq!(read(&[(EXIF_TAG_RATING, TIFF_SHORT, 1, short(&[7]))], &[]).rating, Some(5));
            let percent = (EXIF_TAG_RATING_PERCENT, TIFF_SHORT, 1, short(&[50]));
            assert_eq!(read(std::slice::from_ref(&percent), &[]).rating, Some(3));
            assert_eq!(read(&[(EXIF_TAG_RATING, TIFF_SHORT, 1, short(&[1])), percent], &[]).rating, Some(1));

            // A value pointing outside the block reads as absent; the rest is still read.
            let mut block = tiff_block_as(
                big,
                le,
                &[(EXIF_TAG_RATING, TIFF_SHORT, 1, short(&[2]))],
                &[
                    date(b"2023:07:15 18:30:05"),
                    (EXIF_TAG_DATE_TIME_DIGITIZED, TIFF_ASCII, 20, b"2020:01:02 03:04:05\0".to_vec()),
                ],
            );
            let len = block.len();
            // DateTimeOriginal's offset, in the Exif IFD's first entry: past the end. The IFD0 has two entries.
            let width = if big { 8 } else { 4 };
            let exif_at = if big { 16 + 8 + 20 * 2 + 8 } else { 8 + 2 + 12 * 2 + 4 };
            let count_w = if big { 8 } else { 2 };
            let value = exif_at + count_w + 4 + width;
            block[value..value + width].copy_from_slice(&offset_bytes(big, le, len));
            let m = meta_of(&block, Format::Tiff);
            assert_eq!((m.rating, m.taken), (Some(2), wall_clock_seconds(2020, 1, 2, 3, 4, 5)));
        }
        // The library's case (6 files, which kamadak-exif read no rating from): a block that ends 4 bytes early,
        // cutting off the Exif IFD's next-IFD offset. IFD0's rating is whole, and read.
        let exif_version = (0x9000, TIFF_UNDEFINED, 4, b"0210".to_vec());
        let mut block =
            tiff_block(false, &[(EXIF_TAG_RATING, TIFF_SHORT, 1, 4u16.to_be_bytes().to_vec())], &[exif_version]);
        block.truncate(block.len() - 4);
        assert_eq!(meta_of(&block, Format::Tiff).rating, Some(4));
        // Not a TIFF structure without its 42. A 43 is BigTIFF only with its offset size (8) after it.
        let mut block = tiff_block(true, &[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, 6u16.to_le_bytes().to_vec())], &[]);
        assert_eq!(orientation(&block, Format::Tiff).get(), 6);
        block[2] = 43;
        block[4] = 4;
        assert_eq!(orientation(&block, Format::Tiff).get(), 1);
        // BigTIFF is for a TIFF file: an EXIF block (JPEG, PNG, WebP) with one is not read.
        let big = tiff_block_as(true, true, &[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, 6u16.to_le_bytes().to_vec())], &[]);
        assert!(Exif::read(&big, false).is_none());
        assert_eq!(Exif::read(&big, true).map(|e| e.orientation().get()), Some(6));
    }

    #[test]
    fn bigtiff_reads_as_its_classic_twin() {
        for le in [true, false] {
            let short = |v: u16| (if le { v.to_le_bytes() } else { v.to_be_bytes() }).to_vec();
            let xmp = br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="3"/>"#.to_vec();
            let ifd0 = [
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(6)),
                (EXIF_TAG_RATING, TIFF_SHORT, 1, short(4)),
                (TIFF_TAG_XMP, TIFF_BYTE, u32::try_from(xmp.len()).unwrap(), xmp),
            ];
            let exif = [(EXIF_TAG_DATE_TIME_ORIGINAL, TIFF_ASCII, 20, b"2023:07:15 18:30:05\0".to_vec())];
            let classic = meta_of(&tiff_block(le, &ifd0, &exif), Format::Tiff);
            let big = meta_of(&tiff_block_as(true, le, &ifd0, &exif), Format::Tiff);
            assert_eq!(big, classic);
            // XMP's rating first (3), then the date.
            assert_eq!((big.orientation.get(), big.rating, big.taken), (6, Some(3), Some(1_689_445_805)));
            // The orientation is patched in place, as in a classic file.
            let patched =
                patch_orientation(&tiff_block_as(true, le, &ifd0, &exif), Format::Tiff, Orientation::new(3).unwrap())
                    .unwrap();
            assert_eq!(orientation(&patched, Format::Tiff).get(), 3);
        }
    }

    /// The patcher sets every orientation entry the reader could take, each at its own width, so what it wrote is
    /// what the reader reads back: a BYTE, a LONG, a LONG out of its entry, a repeated tag, and LONG8 in BigTIFF.
    #[test]
    fn orientation_patch_round_trips_every_kind_the_reader_takes() {
        for (big, le) in [(false, true), (false, false), (true, true), (true, false)] {
            let short = |v: &[u16]| -> Vec<u8> {
                v.iter().flat_map(|&v| if le { v.to_le_bytes() } else { v.to_be_bytes() }).collect()
            };
            let long = |v: &[u32]| -> Vec<u8> {
                v.iter().flat_map(|&v| if le { v.to_le_bytes() } else { v.to_be_bytes() }).collect()
            };
            let long8 = |v: &[u64]| -> Vec<u8> {
                v.iter().flat_map(|&v| if le { v.to_le_bytes() } else { v.to_be_bytes() }).collect()
            };
            let mut cases: Vec<Vec<TestEntry>> = vec![
                vec![(EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![6])],
                vec![(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[6]))],
                vec![(EXIF_TAG_ORIENTATION, TIFF_LONG, 1, long(&[6]))],
                // Out of its entry (more than the value field holds): the data it points at is patched, not the
                // pointer.
                vec![(EXIF_TAG_ORIENTATION, TIFF_LONG, 3, long(&[6, 1, 1]))],
                vec![(EXIF_TAG_ORIENTATION, TIFF_SHORT, 3, short(&[6, 1, 1]))],
                // Repeated, of different kinds: both are set, so the last (the one read) reads as patched.
                vec![(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[8])), (EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![6])],
                vec![(EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![8]), (EXIF_TAG_ORIENTATION, TIFF_LONG, 1, long(&[6]))],
            ];
            if big {
                cases.push(vec![(EXIF_TAG_ORIENTATION, TIFF_LONG8, 1, long8(&[6]))]);
                cases.push(vec![(EXIF_TAG_ORIENTATION, TIFF_LONG8, 2, long8(&[6, 1]))]);
            }
            for ifd0 in cases {
                let block = tiff_block_as(big, le, &ifd0, &[]);
                assert_eq!(orientation(&block, Format::Tiff).get(), 6, "{ifd0:?}");
                let patched = patch_orientation(&block, Format::Tiff, Orientation::new(3).unwrap()).unwrap();
                assert_eq!(patched.len(), block.len());
                assert_eq!(orientation(&patched, Format::Tiff).get(), 3, "{ifd0:?}");
                // Every entry was set, the first read too.
                let tiff = Tiff::new(&patched, true).unwrap();
                assert_eq!(tiff.orientation_entries().count(), ifd0.len());
                for (_, e) in tiff.orientation_entries() {
                    assert_eq!(tiff.uint(&e), Some(3), "{ifd0:?}");
                }
                // Nothing but the value changed: a single entry patched back to 6 gives the file it was (the
                // trailing values of an array are kept).
                if ifd0.len() == 1 {
                    let back = patch_orientation(&patched, Format::Tiff, Orientation::new(6).unwrap()).unwrap();
                    assert_eq!(back, block, "{ifd0:?}");
                }
            }
            // An orientation of a kind the reader doesn't take (ASCII) is not patched: there is nothing to set.
            let ascii = tiff_block_as(big, le, &[(EXIF_TAG_ORIENTATION, TIFF_ASCII, 2, b"6\0".to_vec())], &[]);
            assert_eq!(patch_orientation(&ascii, Format::Tiff, Orientation::new(3).unwrap()), None);
        }
    }

    /// A JPEG's EXIF block with a BYTE or a LONG orientation is patched too (the case that turned a picture twice).
    #[test]
    fn jpeg_byte_and_long_orientations_patch() {
        for entry in [
            (EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![6]),
            (EXIF_TAG_ORIENTATION, TIFF_LONG, 1, 6u32.to_be_bytes().to_vec()),
        ] {
            let tiff = tiff_block(false, &[entry], &[]);
            let app1 = [&b"Exif\0\0"[..], &tiff].concat();
            let len = u16::try_from(app1.len() + 2).unwrap().to_be_bytes();
            let jpeg = [&[0xFF, 0xD8, 0xFF, 0xE1][..], &len, &app1, &[0xFF, 0xD9]].concat();
            assert_eq!(orientation(&jpeg, Format::Jpeg).get(), 6);
            let patched = patch_orientation(&jpeg, Format::Jpeg, Orientation::UPRIGHT).unwrap();
            assert_eq!(orientation(&patched, Format::Jpeg), Orientation::UPRIGHT);
        }
    }

    #[test]
    fn exif_dates_as_kamadak_read_them() {
        assert_eq!(exif_date(b"2023:07:15 18:30:05"), Some(1_689_445_805));
        // Anything after the seconds is ignored.
        assert_eq!(exif_date(b"2023:07:15 18:30:05.123+02:00"), Some(1_689_445_805));
        for bad in [
            &b"    :  :     :  :  "[..],
            b"                   ",
            b"0000:00:00 00:00:00",
            b"2023:07:15 18:30:0",
            b"2023-07-15 18:30:05",
            b"2023:07:15T18:30:05",
            b"2023:07:15 18:30:5x",
            b"2023:13:15 18:30:05",
            b"20 3:07:15 18:30:05",
        ] {
            assert_eq!(exif_date(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn taken_from_exif_then_xmp() {
        assert_eq!(taken(&tiff_taken(b"2023:07:15 18:30:05"), Format::Tiff), Some(1_689_445_805));
        // A camera with its clock never set writes a blank date: no date.
        assert_eq!(taken(&tiff_taken(b"0000:00:00 00:00:00"), Format::Tiff), None);

        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/"
            xmp:CreateDate="2020-01-01T00:00:00"><exif:DateTimeOriginal>2019-02-03T04:05:06.78-03:00</exif:DateTimeOriginal>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(png_chunk(b"iTXt", &[&b"XML:com.adobe.xmp\0\0\0\0\0"[..], xmp].concat()));
        png.extend(png_chunk(b"IEND", b""));
        // exif:DateTimeOriginal wins over xmp:CreateDate; the zone is dropped, the wall-clock time kept.
        assert_eq!(taken(&png, Format::Png), wall_clock_seconds(2019, 2, 3, 4, 5, 6));
        assert_eq!(parse_iso_date("2024-02-29"), wall_clock_seconds(2024, 2, 29, 0, 0, 0));
        assert_eq!(parse_iso_date("2023-02-29"), None);
        assert_eq!(parse_iso_date("2024-05-01T10:00Z"), Some(1_714_557_600));
        assert_eq!(wall_clock_seconds(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(wall_clock_seconds(1601, 1, 1, 0, 0, 0), Some(-11_644_473_600));
    }

    fn rating_packet(rating: u8) -> Vec<u8> {
        format!(
            r#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF
            xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="{rating}"/></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#
        )
        .into_bytes()
    }

    #[test]
    fn a_gif_is_read_only_whole() {
        // An XMP application extension after the header: the packet raw, then the 258-byte magic trailer.
        let with_xmp = |gif: &[u8]| {
            let header = gif_header_len(gif).unwrap();
            let mut ext = [&[0x21, 0xFF, 11][..], b"XMP DataXMP", &rating_packet(3), &[0x01]].concat();
            ext.extend((0..=255u8).rev());
            ext.push(0);
            [&gif[..header], &ext, &gif[header..]].concat()
        };
        let still = with_xmp(&gif(1).unwrap());
        assert_eq!((rating(&still, Format::Gif), frames(&still, Format::Gif)), (Some(3), 1));
        let animated = with_xmp(&gif(3).unwrap());
        assert_eq!((rating(&animated, Format::Gif), frames(&animated, Format::Gif)), (Some(3), 3));
        // Without its trailer (a truncated download), nothing it holds is taken: one frame, no rating.
        let truncated = &animated[..animated.len() - 1];
        assert_eq!((rating(truncated, Format::Gif), frames(truncated, Format::Gif)), (None, 1));
    }

    #[test]
    fn a_gif_truncated_anywhere_in_its_xmp_block_reads_without_panicking() {
        // Every cut through the packet and its magic trailer: the block must fit in the file to be read.
        let mut ext = [&[0x21, 0xFF, 11][..], b"XMP DataXMP", &rating_packet(3), &[0x01]].concat();
        ext.extend((0..=255u8).rev());
        ext.push(0);
        let still = gif(1).unwrap();
        let header = gif_header_len(&still).unwrap();
        let whole = [&still[..header], &ext, &still[header..]].concat();
        for cut in 1..=ext.len() {
            let truncated = &whole[..whole.len() - cut];
            assert_eq!(rating(truncated, Format::Gif), None, "cut {cut}");
        }
    }

    #[test]
    fn jpeg_xmp_standard_packet_first() {
        let segment =
            |data: &[u8]| [&[0xFF, 0xE1][..], &u16::try_from(data.len() + 2).unwrap().to_be_bytes(), data].concat();
        // The extended packet (one chunk) comes first in the file, the standard one after it: the standard one is
        // read first.
        let extended = rating_packet(4);
        let said = u32::try_from(extended.len()).unwrap().to_be_bytes();
        let ext_chunk = [JPEG_XMP_EXTENSION, &[b'A'; 32], &said, &0u32.to_be_bytes(), &extended].concat();
        let jpeg = [
            &[0xFF, 0xD8][..],
            &segment(&ext_chunk),
            &segment(&[JPEG_XMP, &rating_packet(2)[..]].concat()),
            &[0xFF, 0xDA, 0, 2],
        ]
        .concat();
        let mut scratch = MetaScratch::default();
        Container::walk(&jpeg, Format::Jpeg, &mut scratch);
        assert!(matches!(scratch.packets[..], [Packet::InFile(_), Packet::Assembled(_)]));
        assert_eq!(rating(&jpeg, Format::Jpeg), Some(2));
    }

    #[test]
    fn frames_of_animations() {
        assert_eq!(frames(&gif(1).unwrap(), Format::Gif), 1);
        assert_eq!(frames(&gif(3).unwrap(), Format::Gif), 3);

        assert_eq!(frames(&apng(1).unwrap(), Format::Png), 1);
        assert_eq!(frames(&apng(4).unwrap(), Format::Png), 4);

        // WebP: the container says it, the VP8X animation flag and one ANMF chunk per frame.
        let animation = |flags: u8, frames: usize| {
            let vp8x = [flags, 0, 0, 0, 1, 0, 0, 1, 0, 0];
            let mut chunks: Vec<(&[u8; 4], &[u8])> = vec![(b"VP8X", &vp8x), (b"ANIM", &[0; 6])];
            chunks.extend(std::iter::repeat_n((b"ANMF", &[0u8; 16][..]), frames));
            webp(&chunks)
        };
        assert_eq!(frames(&animation(0x02, 5), Format::Webp), 5);
        assert_eq!(frames(&animation(0x00, 0), Format::Webp), 1);
    }

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// The next [`xorshift`] value, less than `n`.
    fn below(state: &mut u64, n: usize) -> usize {
        usize::try_from(xorshift(state) % u64::try_from(n).unwrap()).unwrap()
    }

    /// The next [`xorshift`] value's low byte.
    fn byte(state: &mut u64) -> u8 {
        xorshift(state).to_le_bytes()[0]
    }

    /// Extended packets are reassembled as the map of maps did it (the first chunk's length, the last chunk at an
    /// offset, contiguous from 0, by GUID): random chunk lists of up to 3 packets, chunks shuffled, repeated with other
    /// data, missing, or with another length.
    #[test]
    fn extended_packets_as_the_maps_made_them() {
        use std::collections::BTreeMap;
        let mut st = 0x853C49E6748FEA9Bu64;
        let mut scratch = MetaScratch::default();
        for _ in 0..3000 {
            let mut file = Vec::new();
            let mut segments: Vec<Vec<u8>> = Vec::new();
            for g in 0..1 + xorshift(&mut st) % 3 {
                let parity = u8::try_from(xorshift(&mut st) % 2).unwrap();
                let guid = [b'A' + (u8::try_from(g).unwrap() + parity); 32];
                let full = 1 + usize::try_from(xorshift(&mut st) % 40).unwrap();
                let packet: Vec<u8> = (0..full).map(|_| byte(&mut st)).collect();
                let mut at = 0;
                while at < full {
                    let bound = u64::try_from(full - at).unwrap();
                    let len = usize::try_from((1 + xorshift(&mut st) % 12).min(bound)).unwrap();
                    let r = xorshift(&mut st) % 10;
                    let (offset, said, data) = match r {
                        0 => (at + 1, full, packet[at..at + len].to_vec()),
                        1 => (at, full + 1, packet[at..at + len].to_vec()),
                        2 => (at, full, vec![0xEE; len]),
                        _ => (at, full, packet[at..at + len].to_vec()),
                    };
                    if r != 3 || !xorshift(&mut st).is_multiple_of(4) {
                        segments.push(
                            [
                                JPEG_XMP_EXTENSION,
                                &guid,
                                &u32::try_from(said).unwrap().to_be_bytes(),
                                &u32::try_from(offset).unwrap().to_be_bytes(),
                                &data,
                            ]
                            .concat(),
                        );
                    }
                    if r == 2 {
                        // The right data after the wrong: the last at an offset counts.
                        segments.push(
                            [
                                JPEG_XMP_EXTENSION,
                                &guid,
                                &u32::try_from(full).unwrap().to_be_bytes(),
                                &u32::try_from(at).unwrap().to_be_bytes(),
                                &packet[at..at + len],
                            ]
                            .concat(),
                        );
                    }
                    at += len;
                }
            }
            for i in (1..segments.len()).rev() {
                segments.swap(i, below(&mut st, i + 1));
            }
            for seg in &segments {
                file.extend_from_slice(seg);
            }
            // The old way.
            // GUID → (length, chunks by offset).
            type Chunks<'a> = (usize, BTreeMap<usize, &'a [u8]>);
            let mut maps: BTreeMap<&[u8], Chunks> = BTreeMap::new();
            let mut pos = 0;
            let mut ranges = Vec::new();
            for seg in &segments {
                ranges.push(pos..pos + seg.len());
                pos += seg.len();
            }
            for r in &ranges {
                let rest = &file[r.clone()][JPEG_XMP_EXTENSION.len()..];
                let full = pg_model::num::index(u32::from_be_bytes(rest[32..36].try_into().unwrap()));
                let offset = pg_model::num::index(u32::from_be_bytes(rest[36..40].try_into().unwrap()));
                maps.entry(&rest[..32]).or_insert((full, BTreeMap::new())).1.insert(offset, &rest[40..]);
            }
            let want: Vec<Vec<u8>> = maps
                .into_values()
                .filter_map(|(full, chunks)| {
                    let mut packet = Vec::new();
                    for (offset, data) in chunks {
                        if offset != packet.len() {
                            break;
                        }
                        packet.extend_from_slice(data);
                    }
                    (packet.len() == full).then_some(packet)
                })
                .collect();
            // The new.
            scratch.chunks.clear();
            scratch.assembled.clear();
            scratch.packets.clear();
            for r in &ranges {
                add_extended_chunk(&file, &file[r.clone()], &mut scratch.chunks);
            }
            extended_packets(&file, &mut scratch.chunks, &mut scratch.assembled, &mut scratch.packets);
            let got: Vec<Vec<u8>> = (scratch.packets.iter())
                .map(|p| match p {
                    Packet::Assembled(r) => scratch.assembled[r.clone()].to_vec(),
                    Packet::InFile(_) => panic!("an extended packet in the file"),
                })
                .collect();
            assert_eq!(got, want);
        }
    }

    /// A kept decompressor inflates as `decompress_to_vec_zlib` does: the same bytes, the same failures (truncated,
    /// corrupted, empty), each onto what `out` already holds.
    #[test]
    fn an_inflated_packet_over_the_limit_is_absent() {
        // A bomb: 16 MB of zeros packs to a few kilobytes, and inflates past the limit.
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; MAX_INFLATED + 1], 6);
        assert!(zlib.len() < 1 << 20, "{}", zlib.len());
        let mut out = b"before".to_vec();
        assert!(inflate_zlib(&zlib, &mut DecompressorOxide::default(), &mut out, MAX_INFLATED).is_none());
        assert_eq!(out, b"before");
        // Just within the limit: taken.
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; MAX_INFLATED], 6);
        assert!(inflate_zlib(&zlib, &mut DecompressorOxide::default(), &mut out, MAX_INFLATED).is_some());
        // No budget left: absent, and `out` as it was.
        let small = miniz_oxide::deflate::compress_to_vec_zlib(b"<x/>", 6);
        let before = out.len();
        assert!(inflate_zlib(&small, &mut DecompressorOxide::default(), &mut out, 0).is_none());
        assert_eq!(out.len(), before);
    }

    /// The budget is the file's, not each packet's: a PNG of many compressed iTXt chunks, each within the limit
    /// alone, inflates no more than [`MAX_INFLATED`] in all, and the packets past it are absent.
    #[test]
    fn many_compressed_packets_share_one_budget() {
        // Over 4 MiB in each, a few kilobytes packed: 3 fit in the budget, the rest don't.
        let packet = [&b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">"[..], &vec![b' '; 4 << 20], b"</x:xmpmeta>"].concat();
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&packet, 6);
        let itxt = [&b"XML:com.adobe.xmp\0"[..], &[1, 0], b"\0\0", &zlib].concat();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        for _ in 0..10 {
            png.extend(png_chunk(b"iTXt", &itxt));
        }
        png.extend(png_chunk(b"IEND", b""));
        let mut scratch = MetaScratch::default();
        Container::walk(&png, Format::Png, &mut scratch);
        assert!(scratch.assembled.len() <= MAX_INFLATED, "{}", scratch.assembled.len());
        let inflated = scratch.packets.iter().filter(|p| matches!(p, Packet::Assembled(_))).count();
        assert_eq!(inflated, MAX_INFLATED / packet.len());
    }

    #[test]
    fn inflate_as_decompress_to_vec_zlib() {
        let mut st = 0x2545F4914F6CDD1Du64;
        let mut decompressor = Box::<DecompressorOxide>::default();
        let mut out = b"before".to_vec();
        for i in 0..2000 {
            let len = usize::try_from(xorshift(&mut st) % 5000).unwrap();
            let repetitive = i % 2 == 0;
            let data: Vec<u8> =
                (0..len).map(|k| if repetitive { u8::try_from(k % 7).unwrap() } else { byte(&mut st) }).collect();
            let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, u8::try_from(i % 10).unwrap());
            match i % 5 {
                1 if !zlib.is_empty() => zlib.truncate(below(&mut st, zlib.len())),
                2 if !zlib.is_empty() => {
                    let at = below(&mut st, zlib.len());
                    zlib[at] ^= 1 << (xorshift(&mut st) % 8);
                }
                3 => zlib.clear(),
                _ => {}
            }
            let want = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib).ok();
            let before = out.len();
            let got = inflate_zlib(&zlib, &mut decompressor, &mut out, MAX_INFLATED).map(|r| out[r].to_vec());
            assert_eq!(got, want, "case {i}");
            if got.is_none() {
                assert_eq!(out.len(), before);
            }
            assert!(out.starts_with(b"before"));
        }
    }
}
