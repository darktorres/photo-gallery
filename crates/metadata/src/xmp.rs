//! XMP: the packets a file's container holds (kept in [`MetaScratch`] from one file to the next), and the
//! properties read from each: the ratings and the dates. A leaf below `container` and the crate root.

use std::ops::Range;

use miniz_oxide::inflate::core::DecompressorOxide;
use pg_container::ExtendedChunk;
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::ReaderBuffers;

pub(crate) const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_MICROSOFT_PHOTO: &str = "http://ns.microsoft.com/photo/1.0/";
const NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";
const NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";

/// The XMP properties read from each packet: `xmp:Rating`, `MicrosoftPhoto:Rating`, and the dates.
pub(crate) const XMP_WANTED: [(&str, &str); 5] = [
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
    /// The container's XMP packets, main one first ([`Container::walk`](crate::container::Container::walk)).
    pub(crate) packets: Vec<Packet>,
    /// The packets that aren't in the file as they are: reassembled extended JPEG packets, inflated PNG ones.
    pub(crate) assembled: Vec<u8>,
    /// An extended JPEG packet's chunks, as found.
    pub(crate) chunks: Vec<ExtendedChunk>,
    /// The inflater of compressed PNG packets, made when one is first met.
    pub(crate) inflater: Option<Box<DecompressorOxide>>,
    /// The XML reader's.
    pub(crate) xml: ReaderBuffers,
    /// The wanted values of the packet being read, one after another.
    pub(crate) values: String,
}

/// An XMP packet: a range of the file, or of [`MetaScratch::assembled`].
#[derive(Clone)]
pub(crate) enum Packet {
    InFile(Range<usize>),
    Assembled(Range<usize>),
}

/// The first value of each wanted property (namespace URI, local name) in one packet, as an attribute of an
/// `rdf:Description` or as an element's text, trimmed; empty values are skipped. Each is a range of `values`
/// (emptied first), read with the XML reader in `buffers` (given back after).
pub(crate) fn xmp_properties<const N: usize>(
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
        wanted.iter().position(|(uri, name)| {
            matches!(ns, ResolveResult::Bound(Namespace(n)) if same_namespace(n, uri)) && local == *name
        })
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
        let is_open = matches!(event, Event::Start(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let element = which(&ns, e.local_name().as_ref());
                // Only an open element takes the text that follows it: an empty one has no end event, so its name must
                // not reach the text of a later sibling.
                if is_open {
                    inside = element;
                }
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

/// Whether a namespace in a packet is `uri`, with or without its trailing slash: ExifTool writes the Microsoft one
/// without it (`http://ns.microsoft.com/photo/1.0`), and Windows reads that file's percent.
fn same_namespace(n: &str, uri: &str) -> bool {
    n == uri || n == uri.trim_end_matches('/')
}

/// An XMP `xmp:Rating` as the stars Windows shows. A value above 5 shows 5 stars (Windows clamps: an XMP 7 shows 5 stars,
/// measured). -1 (rejected) and anything below 0 read as no rating.
pub(crate) fn xmp_stars(v: f64) -> Option<u8> {
    (v >= 0.0).then(|| round_u8(v).min(5))
}

/// A percent (an XMP `MicrosoftPhoto:Rating` or an EXIF `RatingPercent`) as the stars Windows shows. Windows keeps the
/// value, clamped to 99 (measured: 100 and 101 show 99), and shows the star band of that stored value. Below 0 reads as
/// no rating.
pub(crate) fn xmp_percent_stars(v: f64) -> Option<u8> {
    (v >= 0.0).then(|| stars_of_percent(round_u8(v).min(99)))
}

/// The star count Explorer shows for a stored percent: 0 is unrated, 1 to 12 is one star, 13 to 37 two, 38 to 62 three,
/// 63 to 87 four, and 88 to 99 five. Measured on Explorer's Rating column for every stored value from 0 to 99.
pub(crate) fn stars_of_percent(stored: u8) -> u8 {
    match stored {
        0 => 0,
        1..=12 => 1,
        13..=37 => 2,
        38..=62 => 3,
        63..=87 => 4,
        _ => 5,
    }
}

/// `v` rounded to the nearest whole number, held to 0..=255 (NaN reads as 0).
fn round_u8(v: f64) -> u8 {
    pg_model::num::f64_to_u8(v.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_of_percent_are_explorers_bands() {
        assert_eq!([0, 1, 12, 13, 37, 38, 62, 63, 87, 88, 99].map(stars_of_percent), [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5]);
    }

    #[test]
    fn text_after_an_empty_element_is_not_its_value() {
        let packet = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/"><xmp:Rating/>7</rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let mut buffers = ReaderBuffers::default();
        let mut values = String::new();
        let found = xmp_properties(packet, [(NS_XMP, "Rating")], &mut buffers, &mut values);
        assert_eq!(found, [None]);
    }

    #[test]
    fn stars_clamp_to_five_and_negative_reads_as_none() {
        assert_eq!([0.0, 4.0, 7.0, -1.0].map(xmp_stars), [Some(0), Some(4), Some(5), None]);
        assert_eq!([0.0, 75.0, 101.0, -1.0].map(xmp_percent_stars), [Some(0), Some(4), Some(5), None]);
    }
}
