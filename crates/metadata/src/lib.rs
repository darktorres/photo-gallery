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

mod container;
mod exif;
mod orientation;
#[cfg(test)]
mod test_support;
mod tiff;
mod xmp;

pub use orientation::{orientation, patch_orientation};
pub use xmp::MetaScratch;

use pg_container::range_in;
use pg_model::date::parse_iso_date;
use pg_model::format::Format;
use pg_model::orientation::Orientation;
use pg_model::rating::Rating;

use crate::container::Container;
use crate::exif::{Exif, exif_date};
use crate::xmp::{Packet, XMP_WANTED, percent, xmp_properties, xmp_value};

/// What the import reads from a file's metadata: one walk of its container, one read of its EXIF block, and one
/// parse of each XMP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    /// EXIF orientation 1–8; 1 when absent or unreadable.
    pub orientation: Orientation,
    /// The file's own rating: 0–5 (0 = explicitly unrated), or `None` when it holds none. XMP `xmp:Rating` first
    /// (the standard packet, then the extended one), then `MicrosoftPhoto:Rating`, then EXIF Rating and
    /// RatingPercent. XMP's -1 ("rejected") has no equivalent here and reads as no rating.
    pub rating: Option<Rating>,
    /// When the picture was taken, as wall-clock time (no time zone: what the camera's clock showed) in Unix
    /// seconds: EXIF DateTimeOriginal, then DateTimeDigitized, then XMP `exif:DateTimeOriginal`,
    /// `photoshop:DateCreated` and `xmp:CreateDate`. `None` when the file says nothing.
    pub taken: Option<i64>,
    /// How many frames the file holds: more than one is an animated GIF, PNG (APNG) or WebP. Read from the
    /// container only (no decoding).
    pub frames: u32,
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
            rating: xmp_rating.or(microsoft).or_else(exif_rating).and_then(Rating::new),
            taken: exif.dates.iter().flatten().find_map(|d| exif_date(d)).or(xmp_taken),
            frames: container.frames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::JPEG_XMP;
    use crate::exif::{
        EXIF_TAG_DATE_TIME_DIGITIZED, EXIF_TAG_DATE_TIME_ORIGINAL, EXIF_TAG_EXIF_IFD, EXIF_TAG_RATING,
        EXIF_TAG_RATING_PERCENT, TIFF_TAG_XMP,
    };
    use crate::test_support::png_chunk;
    use crate::tiff::{
        EXIF_TAG_ORIENTATION, TIFF_ASCII, TIFF_BYTE, TIFF_IFD8, TIFF_LONG, TIFF_LONG8, TIFF_SHORT, TIFF_UNDEFINED, Tiff,
    };
    use pg_container::{JPEG_XMP_EXTENSION, gif_header_len};
    use pg_model::date::wall_clock_seconds;
    use pg_testkit::{apng, gif};

    fn meta_of(bytes: &[u8], format: Format) -> Meta {
        Meta::read(bytes, format, &mut MetaScratch::default())
    }

    fn rating(bytes: &[u8], format: Format) -> Option<u8> {
        meta_of(bytes, format).rating.map(Rating::get)
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
            assert_eq!(read(&[(EXIF_TAG_RATING, TIFF_SHORT, 1, short(&[7]))], &[]).rating.map(Rating::get), Some(5));
            let percent = (EXIF_TAG_RATING_PERCENT, TIFF_SHORT, 1, short(&[50]));
            assert_eq!(read(std::slice::from_ref(&percent), &[]).rating.map(Rating::get), Some(3));
            assert_eq!(read(&[(EXIF_TAG_RATING, TIFF_SHORT, 1, short(&[1])), percent], &[]).rating.map(Rating::get), Some(1));

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
            assert_eq!((m.rating.map(Rating::get), m.taken), (Some(2), wall_clock_seconds(2020, 1, 2, 3, 4, 5)));
        }
        // The library's case (6 files, which kamadak-exif read no rating from): a block that ends 4 bytes early,
        // cutting off the Exif IFD's next-IFD offset. IFD0's rating is whole, and read.
        let exif_version = (0x9000, TIFF_UNDEFINED, 4, b"0210".to_vec());
        let mut block =
            tiff_block(false, &[(EXIF_TAG_RATING, TIFF_SHORT, 1, 4u16.to_be_bytes().to_vec())], &[exif_version]);
        block.truncate(block.len() - 4);
        assert_eq!(meta_of(&block, Format::Tiff).rating.map(Rating::get), Some(4));
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
            assert_eq!((big.orientation.get(), big.rating.map(Rating::get), big.taken), (6, Some(3), Some(1_689_445_805)));
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
}
