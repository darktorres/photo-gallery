//! What the import reads of an EXIF block (IFD0's orientation, rating, rating percent and XMP, and the Exif IFD's
//! dates), through the TIFF reader (`tiff`).

use pg_model::date::wall_clock_seconds;
use pg_model::orientation::Orientation;

use crate::tiff::{EXIF_TAG_ORIENTATION, Tiff};

/// An EXIF date, `YYYY:MM:DD HH:MM:SS` (anything after it ignored), as wall-clock seconds; `None` when blank,
/// malformed or impossible. The format is the one kamadak-exif reads; this parser implements it directly.
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
pub(crate) const EXIF_TAG_EXIF_IFD: u16 = 0x8769;
pub(crate) const EXIF_TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
pub(crate) const EXIF_TAG_DATE_TIME_DIGITIZED: u16 = 0x9004;

/// An entry's readable value, by what it holds. Numbers are kept whole: the domain conversions happen in the
/// accessors, so a value that doesn't fit a domain is absent there and never truncated here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Value<'a> {
    /// A BYTE, SHORT, LONG, IFD, LONG8 or IFD8 entry's first value, in full.
    Number(u64),
    /// An ASCII entry's first string (up to its NUL).
    Text(&'a [u8]),
    /// A BYTE or UNDEFINED entry's bytes.
    Bytes(&'a [u8]),
}

/// What the import reads of an EXIF block. Each tag keeps the value of its last readable entry: an entry whose value
/// can't be read is skipped, so the earlier readable entry of the tag stands. Each tag also keeps the readable values
/// that a later entry replaced, in entry order (`superseded`), as ExifTool keeps them under `Tag (1)`, `Tag (2)`, and
/// so on. Nothing reads `superseded` yet; it is kept so the duplicate rule matches ExifTool's.
#[derive(Default)]
pub(crate) struct Exif<'a> {
    orientation: Option<Value<'a>>,
    rating: Option<Value<'a>>,
    rating_percent: Option<Value<'a>>,
    xmp: Option<Value<'a>>,
    exif_ifd: Option<Value<'a>>,
    date_original: Option<Value<'a>>,
    date_digitized: Option<Value<'a>>,
    /// The readable values that a later entry of the same tag replaced, as `(tag, value)`, in entry order.
    pub(crate) superseded: Vec<(u16, Value<'a>)>,
}

impl<'a> Exif<'a> {
    pub(crate) fn read(block: &'a [u8], bigtiff: bool) -> Option<Exif<'a>> {
        let tiff = Tiff::new(block, bigtiff)?;
        let mut exif = Exif::default();
        for e in tiff.entries(tiff.ifd0()?) {
            let value = match e.tag {
                // The orientation kinds the patcher sets too.
                EXIF_TAG_ORIENTATION if tiff.orientation_kind(&e) => tiff.unsigned(&e).map(Value::Number),
                EXIF_TAG_RATING | EXIF_TAG_RATING_PERCENT | EXIF_TAG_EXIF_IFD => tiff.unsigned(&e).map(Value::Number),
                TIFF_TAG_XMP => tiff.bytes(&e).map(Value::Bytes),
                _ => continue,
            };
            exif.keep(e.tag, value);
        }
        // The Exif IFD's position is a file position: a value that doesn't fit a usize is no position.
        if let Some(at) = number(exif.exif_ifd).and_then(|n| usize::try_from(n).ok()) {
            for e in tiff.entries(at) {
                let value = match e.tag {
                    EXIF_TAG_DATE_TIME_ORIGINAL | EXIF_TAG_DATE_TIME_DIGITIZED => tiff.ascii(&e).map(Value::Text),
                    _ => continue,
                };
                exif.keep(e.tag, value);
            }
        }
        Some(exif)
    }

    /// The slot that holds a tag's winning value, if the tag is one this reader keeps.
    fn slot(&mut self, tag: u16) -> Option<&mut Option<Value<'a>>> {
        Some(match tag {
            EXIF_TAG_ORIENTATION => &mut self.orientation,
            EXIF_TAG_RATING => &mut self.rating,
            EXIF_TAG_RATING_PERCENT => &mut self.rating_percent,
            TIFF_TAG_XMP => &mut self.xmp,
            EXIF_TAG_EXIF_IFD => &mut self.exif_ifd,
            EXIF_TAG_DATE_TIME_ORIGINAL => &mut self.date_original,
            EXIF_TAG_DATE_TIME_DIGITIZED => &mut self.date_digitized,
            _ => return None,
        })
    }

    /// Records one entry's value. An entry that doesn't read (`None`) is skipped: it neither replaces the winner nor
    /// is kept. A readable entry becomes the winner, and the winner it replaces is kept in `superseded`.
    fn keep(&mut self, tag: u16, value: Option<Value<'a>>) {
        let (Some(value), Some(slot)) = (value, self.slot(tag)) else { return };
        if let Some(replaced) = slot.replace(value) {
            self.superseded.push((tag, replaced));
        }
    }

    /// The winning orientation, as the EXIF value 1 to 8. A value outside that range is no orientation, and the
    /// picture is shown as stored.
    pub(crate) fn orientation(&self) -> Orientation {
        number(self.orientation)
            .and_then(|o| u8::try_from(o).ok())
            .and_then(Orientation::new)
            .unwrap_or(Orientation::UPRIGHT)
    }

    /// The winning rating field, EXIF `Rating`, as the number it holds. The domain check is the caller's.
    pub(crate) fn rating(&self) -> Option<u64> {
        number(self.rating)
    }

    /// The winning rating percent, EXIF `RatingPercent`, as the number it holds. The domain check is the caller's.
    pub(crate) fn rating_percent(&self) -> Option<u64> {
        number(self.rating_percent)
    }

    /// The winning XMP packet (where a TIFF file keeps it in IFD0).
    pub(crate) fn xmp(&self) -> Option<&'a [u8]> {
        match self.xmp {
            Some(Value::Bytes(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// DateTimeOriginal, then DateTimeDigitized.
    pub(crate) fn dates(&self) -> [Option<&'a [u8]>; 2] {
        [text(self.date_original), text(self.date_digitized)]
    }
}

fn number(value: Option<Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => Some(n),
        _ => None,
    }
}

fn text<'a>(value: Option<Value<'a>>) -> Option<&'a [u8]> {
    match value? {
        Value::Text(text) => Some(text),
        _ => None,
    }
}
