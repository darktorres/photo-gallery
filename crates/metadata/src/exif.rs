//! What the import reads of an EXIF block (IFD0's orientation, rating and XMP, and the Exif IFD's dates), through
//! the TIFF reader (`tiff`).

use pg_model::date::wall_clock_seconds;
use pg_model::orientation::Orientation;

use crate::tiff::{EXIF_TAG_ORIENTATION, Tiff};

/// An EXIF date, `YYYY:MM:DD HH:MM:SS` (anything after it ignored), as wall-clock seconds; `None` when blank,
/// malformed or impossible. The format rules are kamadak-exif's (`DateTime::from_ascii`), which read them before.
pub(crate) fn exif_date(data: &[u8]) -> Option<i64> {
    if data == b"    :  :     :  :  " || data == b"                   " || data.len() < 19 {
        return None;
    }
    if [(4, b':'), (7, b':'), (10, b' '), (13, b':'), (16, b':')].iter().any(|&(i, c)| data.get(i) != Some(&c)) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| {
        let digits = data.get(r)?;
        digits.iter().all(u8::is_ascii_digit).then(|| digits.iter().fold(0, |n, &d| n * 10 + i64::from(d - b'0')))
    };
    wall_clock_seconds(num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?)
}

pub(crate) const EXIF_TAG_RATING: u16 = 0x4746;
pub(crate) const EXIF_TAG_RATING_PERCENT: u16 = 0x4749;
pub(crate) const TIFF_TAG_XMP: u16 = 700;

/// What the import reads of an EXIF block: IFD0's orientation, rating and XMP (where a TIFF file keeps its
/// packet), and the Exif IFD's dates. A field that doesn't fit in the block reads as absent, and the rest are still
/// read; a tag given twice reads as the last one.
#[derive(Default)]
pub(crate) struct Exif<'a> {
    orientation: Option<u32>,
    pub(crate) rating: Option<u32>,
    pub(crate) rating_percent: Option<u32>,
    pub(crate) xmp: Option<&'a [u8]>,
    /// DateTimeOriginal, then DateTimeDigitized.
    pub(crate) dates: [Option<&'a [u8]>; 2],
}

pub(crate) const EXIF_TAG_EXIF_IFD: u16 = 0x8769;
pub(crate) const EXIF_TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
pub(crate) const EXIF_TAG_DATE_TIME_DIGITIZED: u16 = 0x9004;

impl<'a> Exif<'a> {
    pub(crate) fn read(block: &'a [u8], bigtiff: bool) -> Option<Exif<'a>> {
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

    pub(crate) fn orientation(&self) -> Orientation {
        self.orientation.and_then(|o| u8::try_from(o).ok()).map_or(Orientation::UPRIGHT, Orientation::from_tag)
    }
}
