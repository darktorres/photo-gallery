//! Ratings and orientation as the files themselves hold them (PLAN §2: the catalog is the source of truth; the
//! file is read at import and written back by `ingest::writeback`).
//!
//! The rating is read from the file's XMP packet, found where each container keeps it and parsed as XML with
//! namespaces resolved. A byte search could pick up the XMP of an embedded thumbnail or an edit history; this
//! only reads the packet the container declares as the image's own. EXIF Rating/RatingPercent (what Windows
//! writes) are the fallback.

use std::collections::BTreeMap;

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};

use crate::decode::Format;

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

/// The file's own rating: 0–5 (0 = explicitly unrated), or `None` when it holds none. XMP `xmp:Rating` first
/// (the standard packet, then the extended one), then `MicrosoftPhoto:Rating`, then EXIF Rating and RatingPercent.
/// XMP's -1 ("rejected") has no equivalent here and reads as no rating.
pub fn read_rating(bytes: &[u8], format: Format) -> Option<u8> {
    let packets = xmp_packets(bytes, format);
    let mut microsoft = None;
    for packet in &packets {
        let (xmp, ms) = ratings_in_xmp(packet);
        if xmp.is_some() {
            return xmp;
        }
        microsoft = microsoft.or(ms);
    }
    microsoft.or_else(|| exif_rating(bytes))
}

/// The XMP packets a file declares, main one first.
fn xmp_packets(bytes: &[u8], format: Format) -> Vec<Vec<u8>> {
    match format {
        Format::Jpeg => jpeg_xmp(bytes),
        Format::Png => png_chunks(bytes).filter(|c| &c.kind == b"iTXt").filter_map(|c| png_itxt_xmp(c.data)).collect(),
        Format::Webp => webp_chunks(bytes).filter(|c| &c.kind == b"XMP ").map(|c| c.data.to_vec()).collect(),
        Format::Tiff => tiff_xmp(bytes).into_iter().collect(),
        Format::Gif => gif_xmp(bytes).into_iter().collect(),
        Format::Bmp => Vec::new(),
    }
}

/// `xmp:Rating` and `MicrosoftPhoto:Rating` (mapped from percent) of one packet.
fn ratings_in_xmp(packet: &[u8]) -> (Option<u8>, Option<u8>) {
    let [xmp, microsoft] = xmp_properties(packet, [(NS_XMP, "Rating"), (NS_MICROSOFT_PHOTO, "Rating")]);
    let number = |v: Option<String>| v.and_then(|v| v.parse::<f64>().ok());
    (number(xmp).and_then(xmp_value), number(microsoft).map(percent))
}

/// The first value of each wanted property (namespace URI, local name) in one packet, as an attribute of an
/// `rdf:Description` or as an element's text, trimmed; empty values are skipped.
fn xmp_properties<const N: usize>(packet: &[u8], wanted: [(&str, &str); N]) -> [Option<String>; N] {
    let text = String::from_utf8_lossy(packet);
    // The packet wrapper (<?xpacket ...?>) is a processing instruction, which the parser skips.
    let mut reader = NsReader::from_str(&text);
    let mut found: [Option<String>; N] = std::array::from_fn(|_| None);
    // Set while inside a wanted element, to read its text.
    let mut inside: Option<usize> = None;
    let which = |ns: &ResolveResult, local: &str| {
        wanted
            .iter()
            .position(|(uri, name)| matches!(ns, ResolveResult::Bound(Namespace(n)) if n == uri) && local == *name)
    };
    let keep = |found: &mut [Option<String>; N], i: usize, value: &str| {
        let value = value.trim();
        if found[i].is_none() && !value.is_empty() {
            found[i] = Some(value.to_string());
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

fn exif_rating(bytes: &[u8]) -> Option<u8> {
    let exif = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)).ok()?;
    let field = |tag| exif.get_field(exif::Tag(exif::Context::Tiff, tag), exif::In::PRIMARY);
    if let Some(v) = field(EXIF_TAG_RATING).and_then(|f| f.value.get_uint(0)) {
        return Some(v.min(5) as u8);
    }
    field(EXIF_TAG_RATING_PERCENT).and_then(|f| f.value.get_uint(0)).map(|v| percent(v as f64))
}

// --- Date taken ---------------------------------------------------------------------------------------------

/// When the picture was taken, as wall-clock time (no time zone: what the camera's clock showed) in Unix seconds:
/// EXIF DateTimeOriginal, then DateTimeDigitized, then XMP `exif:DateTimeOriginal`, `photoshop:DateCreated` and
/// `xmp:CreateDate`. `None` when the file says nothing.
pub fn read_taken(bytes: &[u8], format: Format) -> Option<i64> {
    exif_taken(bytes, format).or_else(|| {
        xmp_packets(bytes, format).iter().find_map(|packet| {
            let dates = xmp_properties(
                packet,
                [(NS_EXIF, "DateTimeOriginal"), (NS_PHOTOSHOP, "DateCreated"), (NS_XMP, "CreateDate")],
            );
            dates.into_iter().flatten().find_map(|d| parse_iso_date(&d))
        })
    })
}

fn exif_taken(bytes: &[u8], format: Format) -> Option<i64> {
    let block = exif_block(bytes, format)?;
    let exif = exif::Reader::new().read_raw(bytes[block].to_vec()).ok()?;
    [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized].into_iter().find_map(|tag| {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        let exif::Value::Ascii(ref v) = field.value else { return None };
        let d = exif::DateTime::from_ascii(v.first()?).ok()?;
        let n = |v: u8| i64::from(v);
        wall_clock_seconds(i64::from(d.year), n(d.month), n(d.day), n(d.hour), n(d.minute), n(d.second))
    })
}

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

// --- Animation --------------------------------------------------------------------------------------------------

/// How many frames the file holds: more than one is an animated GIF, PNG (APNG) or WebP. Read from the container
/// only (no decoding).
pub fn frame_count(bytes: &[u8], format: Format) -> u32 {
    let n = match format {
        Format::Gif => {
            gif_blocks(bytes).map(|b| b.iter().filter(|b| matches!(b, GifBlock::Image { .. })).count() as u32)
        }
        // acTL comes before the first IDAT.
        Format::Png => png_chunks(bytes)
            .take_while(|c| &c.kind != b"IDAT")
            .find(|c| &c.kind == b"acTL")
            .and_then(|c| Some(u32::from_be_bytes(c.data.get(..4)?.try_into().ok()?))),
        Format::Webp => {
            let animated =
                webp_chunks(bytes).any(|c| &c.kind == b"VP8X" && c.data.first().is_some_and(|f| f & 0x02 != 0));
            animated.then(|| webp_chunks(bytes).filter(|c| &c.kind == b"ANMF").count() as u32)
        }
        Format::Jpeg | Format::Bmp | Format::Tiff => None,
    };
    n.unwrap_or(1).max(1)
}

// --- Containers -------------------------------------------------------------------------------------------------

/// A JPEG marker segment before the image data.
struct Segment<'a> {
    marker: u8,
    /// Offset of the payload (after the length) in the file.
    start: usize,
    data: &'a [u8],
}

/// The marker segments up to the start of scan. Embedded JPEGs (EXIF thumbnails, MPF images) are inside these
/// or after the image, so they are never visited.
fn jpeg_segments(bytes: &[u8]) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let (start, end) = (i + 4, i + 2 + len);
        if len < 2 || end > bytes.len() {
            break;
        }
        out.push(Segment { marker, start, data: &bytes[start..end] });
        i = end;
    }
    out
}

/// Extended XMP being reassembled: its full length, and the chunks received by offset.
type ExtendedXmp<'a> = (usize, BTreeMap<usize, &'a [u8]>);

/// The standard XMP packet, then the extended one reassembled (Adobe XMP Part 3, 1.1.3.1), if complete.
fn jpeg_xmp(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // By GUID.
    let mut extended: BTreeMap<Vec<u8>, ExtendedXmp> = BTreeMap::new();
    for s in jpeg_segments(bytes).into_iter().filter(|s| s.marker == 0xE1) {
        if let Some(packet) = s.data.strip_prefix(JPEG_XMP) {
            out.insert(0, packet.to_vec());
        } else if let Some(rest) = s.data.strip_prefix(JPEG_XMP_EXTENSION)
            && rest.len() >= 40
        {
            let guid = rest[..32].to_vec();
            let full = u32::from_be_bytes(rest[32..36].try_into().unwrap()) as usize;
            let offset = u32::from_be_bytes(rest[36..40].try_into().unwrap()) as usize;
            extended.entry(guid).or_insert((full, BTreeMap::new())).1.insert(offset, &rest[40..]);
        }
    }
    for (full, chunks) in extended.into_values() {
        let mut packet = Vec::with_capacity(full);
        for (offset, data) in chunks {
            if offset != packet.len() {
                break;
            }
            packet.extend_from_slice(data);
        }
        if packet.len() == full {
            out.push(packet);
        }
    }
    out
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

/// The text of an `XML:com.adobe.xmp` iTXt chunk (possibly zlib-compressed).
fn png_itxt_xmp(data: &[u8]) -> Option<Vec<u8>> {
    let rest = data.strip_prefix(b"XML:com.adobe.xmp\0")?;
    let (&compressed, rest) = rest.split_first()?;
    let rest = rest.get(1..)?; // compression method
    let lang_end = rest.iter().position(|&b| b == 0)?;
    let rest = &rest[lang_end + 1..];
    let translated_end = rest.iter().position(|&b| b == 0)?;
    let text = &rest[translated_end + 1..];
    if compressed == 1 { miniz_oxide::inflate::decompress_to_vec_zlib(text).ok() } else { Some(text.to_vec()) }
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

fn tiff_xmp(bytes: &[u8]) -> Option<Vec<u8>> {
    let exif = exif::Reader::new().read_raw(bytes.to_vec()).ok()?;
    let field = exif.get_field(exif::Tag(exif::Context::Tiff, TIFF_TAG_XMP), exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Byte(v) | exif::Value::Undefined(v, _) => Some(v.clone()),
        _ => None,
    }
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

/// The blocks between the header and the trailer.
pub(crate) fn gif_blocks(bytes: &[u8]) -> Option<Vec<GifBlock>> {
    let mut i = gif_header_len(bytes)?;
    let mut out = Vec::new();
    loop {
        match *bytes.get(i)? {
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
                out.push(GifBlock::Extension { label, range: i..end });
                i = end;
            }
            0x2C => {
                let packed = *bytes.get(i + 9)?;
                let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
                let data = i + 10 + table + 1; // LZW minimum code size
                let end = skip_sub_blocks(bytes, data)?;
                out.push(GifBlock::Image { range: i..end });
                i = end;
            }
            0x3B => return Some(out),
            _ => return None,
        }
    }
}

fn gif_xmp(bytes: &[u8]) -> Option<Vec<u8>> {
    gif_blocks(bytes)?.into_iter().find_map(|b| match b {
        GifBlock::Extension { label: 0xFF, range }
            if bytes.get(range.start + 3..range.start + 14) == Some(b"XMP DataXMP") =>
        {
            let data = &bytes[range.start + 14..range.end];
            let end = find(data, b"<?xpacket end=")?;
            let close = find(&data[end..], b"?>")?;
            Some(data[..end + close + 2].to_vec())
        }
        _ => None,
    })
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
        Format::Jpeg => jpeg_segments(bytes)
            .into_iter()
            .find(|s| s.marker == 0xE1 && s.data.starts_with(b"Exif\0\0"))
            .map(|s| s.start + 6..s.start + s.data.len()),
        Format::Png => png_chunks(bytes).find(|c| &c.kind == b"eXIf").map(|c| c.at + 8..c.at + 8 + c.data.len()),
        Format::Webp => webp_chunks(bytes).find(|c| &c.kind == b"EXIF").map(|c| {
            // Some writers keep JPEG's "Exif\0\0" prefix.
            let skip = if c.data.starts_with(b"Exif\0\0") { 6 } else { 0 };
            c.at + 8 + skip..c.at + 8 + c.data.len()
        }),
        Format::Tiff => Some(0..bytes.len()),
        Format::Gif | Format::Bmp => None,
    }
}

/// Offset of the orientation value in a TIFF block's IFD0, if the tag is there as one SHORT.
fn tiff_orientation_offset(tiff: &[u8]) -> Option<usize> {
    let le = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| {
        tiff.get(i..i + 2).map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |i: usize| {
        tiff.get(i..i + 4).map(|b| {
            let b: [u8; 4] = b.try_into().unwrap();
            if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count).map(|k| ifd + 2 + 12 * k).find_map(|e| {
        (u16_at(e)? == EXIF_TAG_ORIENTATION && u16_at(e + 2)? == 3 && u32_at(e + 4)? == 1).then_some(e + 8)
    })
}

/// The file with its EXIF orientation set to `o`, changed in place (same size, nothing moves), or `None` when
/// the file has no orientation tag to change (adding one is a job for ExifTool).
pub fn patch_orientation(bytes: &[u8], format: Format, o: u8) -> Option<Vec<u8>> {
    let block = exif_block(bytes, format)?;
    let at = block.start + tiff_orientation_offset(&bytes[block.clone()])?;
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
        assert_eq!(ratings_in_xmp(attr).0, Some(4));
        // Any prefix bound to the XMP namespace counts; a different namespace with the same prefix doesn't.
        let element = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:a="http://ns.adobe.com/xap/1.0/" xmlns:xmp="urn:other"><xmp:Rating>2</xmp:Rating>
            <a:Rating>5</a:Rating></rdf:Description></rdf:RDF>"#;
        assert_eq!(ratings_in_xmp(element).0, Some(5));
        let windows = br#"<rdf:Description xmlns:MicrosoftPhoto="http://ns.microsoft.com/photo/1.0/"
            MicrosoftPhoto:Rating="75"/>"#;
        assert_eq!(ratings_in_xmp(windows), (None, Some(4)));
        let rejected = br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="-1"/>"#;
        assert_eq!(ratings_in_xmp(rejected).0, None);
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

    #[test]
    fn taken_from_exif_then_xmp() {
        assert_eq!(read_taken(&tiff_taken(b"2023:07:15 18:30:05"), Format::Tiff), Some(1_689_445_805));
        // A camera with its clock never set writes a blank date: no date.
        assert_eq!(read_taken(&tiff_taken(b"0000:00:00 00:00:00"), Format::Tiff), None);

        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/"
            xmp:CreateDate="2020-01-01T00:00:00"><exif:DateTimeOriginal>2019-02-03T04:05:06.78-03:00</exif:DateTimeOriginal>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(png_chunk(b"iTXt", &[&b"XML:com.adobe.xmp\0\0\0\0\0"[..], xmp].concat()));
        png.extend(png_chunk(b"IEND", b""));
        // exif:DateTimeOriginal wins over xmp:CreateDate; the zone is dropped, the wall-clock time kept.
        assert_eq!(read_taken(&png, Format::Png), wall_clock_seconds(2019, 2, 3, 4, 5, 6));
        assert_eq!(parse_iso_date("2024-02-29"), wall_clock_seconds(2024, 2, 29, 0, 0, 0));
        assert_eq!(parse_iso_date("2023-02-29"), None);
        assert_eq!(parse_iso_date("2024-05-01T10:00Z"), Some(1_714_557_600));
        assert_eq!(wall_clock_seconds(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(wall_clock_seconds(1601, 1, 1, 0, 0, 0), Some(-11_644_473_600));
    }

    #[test]
    fn frames_of_animations() {
        use image::codecs::gif::GifEncoder;
        let gif = |frames: usize| {
            let mut out = Vec::new();
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..frames {
                let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([i as u8 * 60, 0, 0, 255]));
                enc.encode_frame(image::Frame::new(img)).unwrap();
            }
            drop(enc);
            out
        };
        assert_eq!(frame_count(&gif(1), Format::Gif), 1);
        assert_eq!(frame_count(&gif(3), Format::Gif), 3);

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
        assert_eq!(frame_count(&apng(1), Format::Png), 1);
        assert_eq!(frame_count(&apng(4), Format::Png), 4);

        // WebP: the container says it, the VP8X animation flag and one ANMF chunk per frame.
        let webp = |flags: u8, frames: usize| {
            let chunk = |kind: &[u8; 4], data: &[u8]| [&kind[..], &(data.len() as u32).to_le_bytes(), data].concat();
            let mut body = b"WEBP".to_vec();
            body.extend(chunk(b"VP8X", &[flags, 0, 0, 0, 1, 0, 0, 1, 0, 0]));
            body.extend(chunk(b"ANIM", &[0; 6]));
            for _ in 0..frames {
                body.extend(chunk(b"ANMF", &[0; 16]));
            }
            [&b"RIFF"[..], &(body.len() as u32).to_le_bytes(), &body].concat()
        };
        assert_eq!(frame_count(&webp(0x02, 5), Format::Webp), 5);
        assert_eq!(frame_count(&webp(0x00, 0), Format::Webp), 1);
    }
}
