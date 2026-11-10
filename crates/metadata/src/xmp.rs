//! XMP: the packets a file's container holds (kept in [`MetaScratch`] from one file to the next), and the
//! properties read from each: the ratings and the dates. A leaf below `container` and the crate root.

use std::ops::Range;

use miniz_oxide::inflate::core::DecompressorOxide;
use pg_container::ExtendedChunk;
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::ReaderBuffers;

const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
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
pub(crate) fn xmp_value(v: f64) -> Option<u8> {
    (0.0..=5.0).contains(&v).then(|| round_u8(v))
}

/// Windows' percentage scale (1, 25, 50, 75, 99), as ImageRater mapped it.
pub(crate) fn percent(v: f64) -> u8 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_scale() {
        assert_eq!([0.0, 1.0, 25.0, 50.0, 75.0, 99.0, 100.0].map(percent), [0, 1, 2, 3, 4, 5, 5]);
    }
}
