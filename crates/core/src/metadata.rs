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

/// `xmp:Rating` and `MicrosoftPhoto:Rating` (mapped from percent) of one packet, as attributes of an
/// `rdf:Description` or as elements.
fn ratings_in_xmp(packet: &[u8]) -> (Option<u8>, Option<u8>) {
    let text = String::from_utf8_lossy(packet);
    // The packet wrapper (<?xpacket ...?>) is a processing instruction, which the parser skips.
    let mut reader = NsReader::from_str(&text);
    let (mut xmp, mut microsoft) = (None, None);
    // Set while inside a rating element, to read its text.
    let mut inside: Option<bool> = None;
    while let Ok((ns, event)) = reader.read_resolved_event() {
        let which = |ns: &ResolveResult, local: &str| match ns {
            ResolveResult::Bound(Namespace(n)) if local == "Rating" && *n == NS_XMP => Some(true),
            ResolveResult::Bound(Namespace(n)) if local == "Rating" && *n == NS_MICROSOFT_PHOTO => Some(false),
            _ => None,
        };
        match event {
            Event::Start(e) | Event::Empty(e) => {
                inside = which(&ns, e.local_name().as_ref());
                for attr in e.attributes().flatten() {
                    let (ans, local) = reader.resolver().resolve_attribute(attr.key);
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .ok()
                        .and_then(|v| v.trim().parse::<f64>().ok());
                    match (which(&ans, local.as_ref()), value) {
                        (Some(true), Some(v)) => xmp = xmp.or(Some(xmp_value(v))),
                        (Some(false), Some(v)) => microsoft = microsoft.or(Some(percent(v))),
                        _ => {}
                    }
                }
            }
            Event::Text(t) => {
                let value = quick_xml::escape::unescape(&t).ok().and_then(|v| v.trim().parse::<f64>().ok());
                match (inside, value) {
                    (Some(true), Some(v)) => xmp = xmp.or(Some(xmp_value(v))),
                    (Some(false), Some(v)) => microsoft = microsoft.or(Some(percent(v))),
                    _ => {}
                }
            }
            Event::End(_) => inside = None,
            Event::Eof => break,
            _ => {}
        }
    }
    (xmp.flatten(), microsoft)
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

fn orientation_from(mirrored: bool, turns: u8) -> u8 {
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
}
