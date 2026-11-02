//! Ratings, dates, orientation and frame counts as the files themselves hold them (PLAN §2: the catalog is the
//! source of truth; the file is read at import, [`Meta::read`], and written back by `ingest::writeback`).
//!
//! The rating is read from the file's XMP packet, found where each container keeps it and parsed as XML with
//! namespaces resolved. A byte search could pick up the XMP of an embedded thumbnail or an edit history; this
//! only reads the packet the container declares as the image's own. EXIF Rating/RatingPercent (what Windows
//! writes) are the fallback.
//!
//! EXIF is read in place by a reader of its own: IFD0 and the Exif IFD, only the tags used here, every offset
//! checked. A broken field elsewhere in the block (a thumbnail's IFD, GPS) doesn't lose the ones read.

use std::ops::Range;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::ReaderBuffers;

use crate::format::Format;
const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_MICROSOFT_PHOTO: &str = "http://ns.microsoft.com/photo/1.0/";
const NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";
const NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const JPEG_XMP_EXTENSION: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";
const EXIF_TAG_RATING: u16 = 0x4746;
const EXIF_TAG_RATING_PERCENT: u16 = 0x4749;
const EXIF_TAG_ORIENTATION: u16 = 0x0112;
const TIFF_TAG_XMP: u16 = 700;

/// Whether the format can carry a rating (an XMP packet) that other programs read. BMP can't: its rating lives in
/// the catalog only.
pub fn can_hold_rating(format: Format) -> bool {
    !matches!(format, Format::Bmp)
}

/// What the import reads from a file's metadata: one walk of its container, one read of its EXIF block, and one
/// parse of each XMP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    /// EXIF orientation 1–8; 1 when absent or unreadable.
    pub orientation: u8,
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

/// Where `part`, a slice of `bytes`, is in it.
fn range_in(bytes: &[u8], part: &[u8]) -> Range<usize> {
    let start = part.as_ptr() as usize - bytes.as_ptr() as usize;
    debug_assert!(start + part.len() <= bytes.len());
    start..start + part.len()
}

impl Meta {
    pub fn read(bytes: &[u8], format: Format, scratch: &mut MetaScratch) -> Meta {
        let container = Container::walk(bytes, format, scratch);
        let exif = container
            .exif
            .clone()
            .and_then(|block| Exif::read(&bytes[block], format == Format::Tiff))
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
                Packet::InFile(r) => &bytes[r.clone()],
                Packet::Assembled(r) => &assembled[r.clone()],
            };
            let found = xmp_properties(packet, XMP_WANTED, xml, values);
            let [rating, ms, original, created, create] = found.map(|v| v.map(|r| &values[r]));
            let number = |v: Option<&str>| v.and_then(|v| v.parse::<f64>().ok());
            xmp_rating = xmp_rating.or_else(|| number(rating).and_then(xmp_value));
            microsoft = microsoft.or_else(|| number(ms).map(percent));
            xmp_taken =
                xmp_taken.or_else(|| [original, created, create].into_iter().flatten().find_map(parse_iso_date));
        }
        let exif_rating =
            || exif.rating.map(|v| v.min(5) as u8).or_else(|| exif.rating_percent.map(|v| percent(v as f64)));
        Meta {
            orientation: exif.orientation(),
            rating: xmp_rating.or(microsoft).or_else(exif_rating),
            taken: exif.dates.iter().flatten().find_map(|d| exif_date(d)).or(xmp_taken),
            frames: container.frames,
        }
    }
}

/// The EXIF orientation (1–8) of a file, alone; 1 when absent or unreadable.
pub fn orientation(bytes: &[u8], format: Format) -> u8 {
    exif_block(bytes, format)
        .and_then(|block| Exif::read(&bytes[block], format == Format::Tiff))
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
        if found[i].is_none() && !value.is_empty() {
            let start = values.len();
            values.push_str(value);
            found[i] = Some(start..values.len());
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
    (0.0..=5.0).contains(&v).then(|| v.round() as u8)
}

/// Windows' percentage scale (1, 25, 50, 75, 99), as ImageRater mapped it.
fn percent(v: f64) -> u8 {
    match v.round() as i64 {
        ..=0 => 0,
        1..=24 => 1,
        25..=49 => 2,
        50..=74 => 3,
        75..=98 => 4,
        _ => 5,
    }
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
        if self.big { usize::try_from(self.u64_at(i)?).ok() } else { Some(self.u32_at(i)? as usize) }
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
        self.data.get(start..start.checked_add(len)?)
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
                EXIF_TAG_ORIENTATION => exif.orientation = tiff.uint(&e),
                EXIF_TAG_RATING => exif.rating = tiff.uint(&e),
                EXIF_TAG_RATING_PERCENT => exif.rating_percent = tiff.uint(&e),
                TIFF_TAG_XMP => exif.xmp = tiff.bytes(&e),
                EXIF_TAG_EXIF_IFD => exif_ifd = tiff.uint(&e),
                _ => {}
            }
        }
        for e in exif_ifd.into_iter().flat_map(|ifd| tiff.entries(ifd as usize)) {
            match e.tag {
                EXIF_TAG_DATE_TIME_ORIGINAL => exif.dates[0] = tiff.ascii(&e),
                EXIF_TAG_DATE_TIME_DIGITIZED => exif.dates[1] = tiff.ascii(&e),
                _ => {}
            }
        }
        Some(exif)
    }

    fn orientation(&self) -> u8 {
        self.orientation.filter(|o| (1..=8).contains(o)).map_or(1, |o| o as u8)
    }
}

/// An EXIF date, `YYYY:MM:DD HH:MM:SS` (anything after it ignored), as wall-clock seconds; `None` when blank,
/// malformed or impossible. The format rules are kamadak-exif's (`DateTime::from_ascii`), which read them before.
fn exif_date(data: &[u8]) -> Option<i64> {
    if data == b"    :  :     :  :  " || data == b"                   " || data.len() < 19 {
        return None;
    }
    if [(4, b':'), (7, b':'), (10, b' '), (13, b':'), (16, b':')].iter().any(|&(i, c)| data[i] != c) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| {
        let digits = &data[r];
        digits.iter().all(u8::is_ascii_digit).then(|| digits.iter().fold(0, |n, &d| n * 10 + i64::from(d - b'0')))
    };
    wall_clock_seconds(num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?)
}

// --- Dates ------------------------------------------------------------------------------------------------------

/// `YYYY-MM-DD`, optionally followed by `THH:MM[:SS[.fff]]` and a zone (ignored: the wall-clock time is kept).
pub fn parse_iso_date(s: &str) -> Option<i64> {
    let num = |s: &str| -> Option<i64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let (date, time) = s.trim().split_once('T').unwrap_or((s.trim(), ""));
    let mut d = date.splitn(3, '-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (mut h, mut mi, mut sec) = (0, 0, 0);
    if !time.is_empty() {
        let clock = time.split(['Z', '+', '-']).next()?;
        let mut t = clock.splitn(3, ':');
        h = num(t.next()?)?;
        mi = num(t.next()?)?;
        if let Some(s) = t.next() {
            sec = num(s.split('.').next()?)?;
        }
    }
    wall_clock_seconds(y, m, day, h, mi, sec)
}

/// Seconds since 1970-01-01 00:00 of a calendar date and time (proleptic Gregorian, no time zone); `None` for an
/// impossible one (EXIF's blank `0000:00:00 00:00:00` included).
pub fn wall_clock_seconds(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > month_days[m as usize - 1] {
        return None;
    }
    if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..=60).contains(&s) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
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
                                packets.extend(inflate_zlib(text, inflater, assembled).map(Packet::Assembled));
                            }
                            None => {}
                        },
                        b"IDAT" => image_data = true,
                        b"acTL" if !image_data && animation.is_none() => {
                            animation = Some(chunk.data.get(..4).map(|n| u32::from_be_bytes(n.try_into().unwrap())));
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

/// A JPEG marker segment before the image data.
struct Segment<'a> {
    marker: u8,
    /// Offset of the payload (after the length) in the file.
    start: usize,
    data: &'a [u8],
}

/// The marker segments up to the start of scan. Embedded JPEGs (EXIF thumbnails, MPF images) are inside these
/// or after the image, so they are never visited.
fn jpeg_segments(bytes: &[u8]) -> impl Iterator<Item = Segment<'_>> {
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
fn jpeg_exif(s: &Segment) -> Option<std::ops::Range<usize>> {
    (s.marker == 0xE1 && s.data.starts_with(b"Exif\0\0")).then(|| s.start + 6..s.start + s.data.len())
}

/// A chunk of an extended XMP packet (Adobe XMP Part 3, 1.1.3.1), as found in a JPEG.
struct ExtendedChunk {
    /// Its packet's GUID, in the file.
    guid: Range<usize>,
    /// Its packet's length, as this chunk says.
    full: usize,
    offset: usize,
    data: Range<usize>,
    /// The order it was found in.
    seq: usize,
}

/// An APP1 segment's data, if it is a chunk of an extended packet.
fn add_extended_chunk(bytes: &[u8], data: &[u8], chunks: &mut Vec<ExtendedChunk>) {
    if let Some(rest) = data.strip_prefix(JPEG_XMP_EXTENSION)
        && rest.len() >= 40
    {
        let full = u32::from_be_bytes(rest[32..36].try_into().unwrap()) as usize;
        let offset = u32::from_be_bytes(rest[36..40].try_into().unwrap()) as usize;
        let (guid, data) = (range_in(bytes, &rest[..32]), range_in(bytes, &rest[40..]));
        chunks.push(ExtendedChunk { guid, full, offset, data, seq: chunks.len() });
    }
}

/// The extended packets whose chunks are all there, by GUID, each reassembled into `assembled`. A packet's length
/// is what its first chunk found says; of chunks at the same offset, the last found counts; the chunks must follow
/// each other from offset 0 up to that length.
fn extended_packets(bytes: &[u8], chunks: &mut [ExtendedChunk], assembled: &mut Vec<u8>, packets: &mut Vec<Packet>) {
    chunks.sort_unstable_by(|a, b| {
        (&bytes[a.guid.clone()], a.offset, a.seq).cmp(&(&bytes[b.guid.clone()], b.offset, b.seq))
    });
    for packet in chunks.chunk_by(|a, b| bytes[a.guid.clone()] == bytes[b.guid.clone()]) {
        let full = packet.iter().min_by_key(|c| c.seq).unwrap().full;
        let start = assembled.len();
        for at_offset in packet.chunk_by(|a, b| a.offset == b.offset) {
            let last = at_offset.last().unwrap();
            if last.offset != assembled.len() - start {
                break;
            }
            assembled.extend_from_slice(&bytes[last.data.clone()]);
        }
        if assembled.len() - start == full {
            packets.push(Packet::Assembled(start..assembled.len()));
        } else {
            assembled.truncate(start);
        }
    }
}

/// zlib data inflated onto the end of `out`, as `miniz_oxide::inflate::decompress_to_vec_zlib` would give it (the
/// same output, the same failures), with a kept decompressor. Its range in `out`; `None`, and `out` as it was, if it
/// doesn't inflate.
fn inflate_zlib(mut input: &[u8], decompressor: &mut DecompressorOxide, out: &mut Vec<u8>) -> Option<Range<usize>> {
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    decompressor.init();
    let start = out.len();
    out.resize(start + input.len().saturating_mul(2), 0);
    let mut at = 0;
    loop {
        let (status, read, written) = decompress(decompressor, input, &mut out[start..], at, flags);
        at += written;
        match status {
            TINFLStatus::Done => {
                out.truncate(start + at);
                return Some(start..out.len());
            }
            TINFLStatus::HasMoreOutput if read <= input.len() => {
                input = &input[read..];
                let len = out.len() - start;
                out.resize(start + len.saturating_mul(2), 0);
            }
            _ => {
                out.truncate(start);
                return None;
            }
        }
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
    let rest = &rest[lang_end + 1..];
    let translated_end = rest.iter().position(|&b| b == 0)?;
    let text = &rest[translated_end + 1..];
    Some(if compressed == 1 { ItxtXmp::Compressed(text) } else { ItxtXmp::Plain(text) })
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
fn chunk_data(c: &Chunk) -> std::ops::Range<usize> {
    c.at + 8..c.at + 8 + c.data.len()
}

/// A WebP EXIF chunk's EXIF block. Some writers keep JPEG's "Exif\0\0" prefix.
fn webp_exif(c: &Chunk) -> std::ops::Range<usize> {
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

/// The XMP packet of an application extension (`range`: the whole block), if it holds one.
fn gif_xmp(bytes: &[u8], range: std::ops::Range<usize>) -> Option<&[u8]> {
    if bytes.get(range.start + 3..range.start + 14) != Some(b"XMP DataXMP") {
        return None;
    }
    let data = &bytes[range.start + 14..range.end];
    let end = find(data, b"<?xpacket end=")?;
    let close = find(&data[end..], b"?>")?;
    Some(&data[..end + close + 2])
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// --- Orientation ------------------------------------------------------------------------------------------------

/// EXIF orientation (1–8) as (mirrored, quarter turns clockwise): the upright image is the stored one mirrored
/// left-right (if so), then turned.
fn orientation_parts(o: u8) -> (bool, u8) {
    match o {
        2 => (true, 0),
        3 => (false, 2),
        4 => (true, 2),
        5 => (true, 3),
        6 => (false, 1),
        7 => (true, 1),
        8 => (false, 3),
        _ => (false, 0),
    }
}

pub(crate) fn orientation_from(mirrored: bool, turns: u8) -> u8 {
    match (mirrored, turns % 4) {
        (false, 0) => 1,
        (true, 0) => 2,
        (false, 2) => 3,
        (true, 2) => 4,
        (true, 3) => 5,
        (false, 1) => 6,
        (true, 1) => 7,
        _ => 8,
    }
}

/// The orientation that shows the image `turns` quarter turns further clockwise than `o` does.
pub fn compose_orientation(o: u8, turns: u8) -> u8 {
    let (mirrored, t) = orientation_parts(o);
    orientation_from(mirrored, t + turns)
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

/// Offset of the orientation value in a TIFF block's IFD0, if the tag is there as one SHORT.
fn tiff_orientation_offset(block: &[u8], bigtiff: bool) -> Option<usize> {
    let tiff = Tiff::new(block, bigtiff)?;
    tiff.entries(tiff.ifd0()?)
        .find(|e| e.tag == EXIF_TAG_ORIENTATION && e.kind == TIFF_SHORT && e.count == 1)
        .map(|e| tiff.value_at(&e))
}

/// The file with its EXIF orientation set to `o`, changed in place (same size, nothing moves), or `None` when
/// the file has no orientation tag to change (adding one is a job for ExifTool).
pub fn patch_orientation(bytes: &[u8], format: Format, o: u8) -> Option<Vec<u8>> {
    let block = exif_block(bytes, format)?;
    let at = block.start + tiff_orientation_offset(&bytes[block.clone()], format == Format::Tiff)?;
    let le = bytes.get(block.start..block.start + 2)? == b"II";
    let mut out = bytes.to_vec();
    out[at..at + 2].copy_from_slice(&if le { (o as u16).to_le_bytes() } else { (o as u16).to_be_bytes() });
    if format == Format::Png {
        // The eXIf chunk's CRC covers its type and data.
        let chunk = png_chunks(&out).find(|c| &c.kind == b"eXIf")?;
        let (start, end) = (chunk.at + 4, chunk.at + 8 + chunk.data.len());
        let crc = crc32fast::hash(&out[start..end]);
        out[end..end + 4].copy_from_slice(&crc.to_be_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            body.extend([&kind[..], &(data.len() as u32).to_le_bytes(), data].concat());
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        [&b"RIFF"[..], &(body.len() as u32).to_le_bytes(), &body].concat()
    }

    #[test]
    fn composes_orientations() {
        // Four quarter turns are the identity; one turn of the identity is EXIF 6 (90° clockwise).
        for o in 1..=8 {
            assert_eq!(compose_orientation(o, 4), o);
            assert_eq!(compose_orientation(o, 0), o);
        }
        assert_eq!(compose_orientation(1, 1), 6);
        assert_eq!(compose_orientation(6, 1), 3);
        assert_eq!(compose_orientation(1, 3), 8);
        assert_eq!(compose_orientation(2, 1), 7);
    }

    #[test]
    fn composition_matches_turning_the_upright_image() {
        use crate::decode::{Rgb, orient};
        let img = || Rgb { width: 3, height: 2, pixels: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        for o in 1..=8 {
            // Turning the upright image clockwise is EXIF 6 applied to it.
            let expected = orient(orient(img(), o), 6);
            let got = orient(img(), compose_orientation(o, 1));
            assert_eq!(
                (got.width, got.height, &got.pixels),
                (expected.width, expected.height, &expected.pixels),
                "o = {o}"
            );
        }
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
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
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
        let offset = |v: usize| if big { u64b(v as u64).to_vec() } else { u32b(v as u32).to_vec() };
        // The IFD's entry count is 2 bytes, or 8 in BigTIFF; an entry is 12, or 20.
        let (count_w, entry) = if big { (8, 20) } else { (2, 12) };
        let head = if big { 16 } else { 8 };
        let ifd_len = |n: usize| count_w + entry * n + width;
        let n0 = ifd0.len() + usize::from(!exif.is_empty());
        let exif_at = head + ifd_len(n0);
        let mut data_at = exif_at + if exif.is_empty() { 0 } else { ifd_len(exif.len()) };
        let mut data: Vec<u8> = Vec::new();
        let mut ifd = |entries: &[TestEntry], out: &mut Vec<u8>| {
            out.extend(if big { u64b(entries.len() as u64).to_vec() } else { u16b(entries.len() as u16).to_vec() });
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
                (TIFF_IFD8, u64b(exif_at as u64).to_vec())
            } else {
                (TIFF_LONG, u32b(exif_at as u32).to_vec())
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
            (false, true) => (v as u32).to_le_bytes().to_vec(),
            (false, false) => (v as u32).to_be_bytes().to_vec(),
            (true, true) => (v as u64).to_le_bytes().to_vec(),
            (true, false) => (v as u64).to_be_bytes().to_vec(),
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
            assert_eq!((m.orientation, m.taken), (6, Some(1_689_445_805)));
            // A SHORT array stored out of the entry: its first value. BYTE and LONG count too; other types don't.
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 3, short(&[8, 1, 1]))], &[]).orientation, 8);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_BYTE, 1, vec![3])], &[]).orientation, 3);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_LONG, 1, long(5))], &[]).orientation, 5);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_ASCII, 2, b"6\0".to_vec())], &[]).orientation, 1);
            assert_eq!(read(&[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[9]))], &[]).orientation, 1);
            // A tag given twice reads as the last.
            let twice = [
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[6])),
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(&[3])),
            ];
            assert_eq!(read(&twice, &[]).orientation, 3);
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
        assert_eq!(orientation(&block, Format::Tiff), 6);
        block[2] = 43;
        block[4] = 4;
        assert_eq!(orientation(&block, Format::Tiff), 1);
        // BigTIFF is for a TIFF file: an EXIF block (JPEG, PNG, WebP) with one is not read.
        let big = tiff_block_as(true, true, &[(EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, 6u16.to_le_bytes().to_vec())], &[]);
        assert!(Exif::read(&big, false).is_none());
        assert_eq!(Exif::read(&big, true).map(|e| e.orientation()), Some(6));
    }

    #[test]
    fn bigtiff_reads_as_its_classic_twin() {
        for le in [true, false] {
            let short = |v: u16| (if le { v.to_le_bytes() } else { v.to_be_bytes() }).to_vec();
            let xmp = br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="3"/>"#.to_vec();
            let ifd0 = [
                (EXIF_TAG_ORIENTATION, TIFF_SHORT, 1, short(6)),
                (EXIF_TAG_RATING, TIFF_SHORT, 1, short(4)),
                (TIFF_TAG_XMP, TIFF_BYTE, xmp.len() as u32, xmp),
            ];
            let exif = [(EXIF_TAG_DATE_TIME_ORIGINAL, TIFF_ASCII, 20, b"2023:07:15 18:30:05\0".to_vec())];
            let classic = meta_of(&tiff_block(le, &ifd0, &exif), Format::Tiff);
            let big = meta_of(&tiff_block_as(true, le, &ifd0, &exif), Format::Tiff);
            assert_eq!(big, classic);
            // XMP's rating first (3), then the date.
            assert_eq!((big.orientation, big.rating, big.taken), (6, Some(3), Some(1_689_445_805)));
            // The orientation is patched in place, as in a classic file.
            let patched = patch_orientation(&tiff_block_as(true, le, &ifd0, &exif), Format::Tiff, 3).unwrap();
            assert_eq!(orientation(&patched, Format::Tiff), 3);
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

    fn gif(frames: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for i in 0..frames {
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([i as u8 * 60, 0, 0, 255]));
            enc.encode_frame(image::Frame::new(img)).unwrap();
        }
        drop(enc);
        out
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
        let still = with_xmp(&gif(1));
        assert_eq!((rating(&still, Format::Gif), frames(&still, Format::Gif)), (Some(3), 1));
        let animated = with_xmp(&gif(3));
        assert_eq!((rating(&animated, Format::Gif), frames(&animated, Format::Gif)), (Some(3), 3));
        // Without its trailer (a truncated download), nothing it holds is taken: one frame, no rating.
        let truncated = &animated[..animated.len() - 1];
        assert_eq!((rating(truncated, Format::Gif), frames(truncated, Format::Gif)), (None, 1));
    }

    #[test]
    fn jpeg_xmp_standard_packet_first() {
        let segment = |data: &[u8]| [&[0xFF, 0xE1][..], &((data.len() + 2) as u16).to_be_bytes(), data].concat();
        // The extended packet (one chunk) comes first in the file, the standard one after it: the standard one is
        // read first.
        let extended = rating_packet(4);
        let ext_chunk =
            [JPEG_XMP_EXTENSION, &[b'A'; 32], &(extended.len() as u32).to_be_bytes(), &0u32.to_be_bytes(), &extended]
                .concat();
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
        assert_eq!(frames(&gif(1), Format::Gif), 1);
        assert_eq!(frames(&gif(3), Format::Gif), 3);

        let apng = |frames: u32| {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgb);
            if frames > 1 {
                enc.set_animated(frames, 0).unwrap();
            }
            let mut w = enc.write_header().unwrap();
            for _ in 0..frames {
                w.write_image_data(&[7; 12]).unwrap();
            }
            w.finish().unwrap();
            out
        };
        assert_eq!(frames(&apng(1), Format::Png), 1);
        assert_eq!(frames(&apng(4), Format::Png), 4);

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
                let guid = [b'A' + (g as u8 + (xorshift(&mut st) % 2) as u8); 32];
                let full = 1 + (xorshift(&mut st) % 40) as usize;
                let packet: Vec<u8> = (0..full).map(|_| xorshift(&mut st) as u8).collect();
                let mut at = 0;
                while at < full {
                    let len = (1 + xorshift(&mut st) % 12).min((full - at) as u64) as usize;
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
                                &(said as u32).to_be_bytes(),
                                &(offset as u32).to_be_bytes(),
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
                                &(full as u32).to_be_bytes(),
                                &(at as u32).to_be_bytes(),
                                &packet[at..at + len],
                            ]
                            .concat(),
                        );
                    }
                    at += len;
                }
            }
            for i in (1..segments.len()).rev() {
                segments.swap(i, xorshift(&mut st) as usize % (i + 1));
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
                let full = u32::from_be_bytes(rest[32..36].try_into().unwrap()) as usize;
                let offset = u32::from_be_bytes(rest[36..40].try_into().unwrap()) as usize;
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
    fn inflate_as_decompress_to_vec_zlib() {
        let mut st = 0x2545F4914F6CDD1Du64;
        let mut decompressor = Box::<DecompressorOxide>::default();
        let mut out = b"before".to_vec();
        for i in 0..2000 {
            let len = (xorshift(&mut st) % 5000) as usize;
            let repetitive = i % 2 == 0;
            let data: Vec<u8> =
                (0..len).map(|k| if repetitive { (k % 7) as u8 } else { xorshift(&mut st) as u8 }).collect();
            let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, (i % 10) as u8);
            match i % 5 {
                1 if !zlib.is_empty() => zlib.truncate(xorshift(&mut st) as usize % zlib.len()),
                2 if !zlib.is_empty() => {
                    let at = xorshift(&mut st) as usize % zlib.len();
                    zlib[at] ^= 1 << (xorshift(&mut st) % 8);
                }
                3 => zlib.clear(),
                _ => {}
            }
            let want = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib).ok();
            let before = out.len();
            let got = inflate_zlib(&zlib, &mut decompressor, &mut out).map(|r| out[r].to_vec());
            assert_eq!(got, want, "case {i}");
            if got.is_none() {
                assert_eq!(out.len(), before);
            }
            assert!(out.starts_with(b"before"));
        }
    }
}
