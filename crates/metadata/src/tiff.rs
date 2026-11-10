//! A TIFF structure (an EXIF block, or a TIFF file), read in place: its IFDs' entries and their values, every
//! offset checked. A leaf: `exif` reads its tags through it, and `orientation` patches IFD0's orientation in it.

use std::ops::Range;

/// IFD0's orientation tag (the one entry the reader and the patcher both look for).
pub(crate) const EXIF_TAG_ORIENTATION: u16 = 0x0112;

/// A TIFF structure (an EXIF block, or a TIFF file), read in place: every offset is checked against it.
pub(crate) struct Tiff<'a> {
    data: &'a [u8],
    pub(crate) little_endian: bool,
    /// BigTIFF: 8-byte counts and offsets, 20-byte entries, an 8-byte value field.
    big: bool,
}

/// An IFD entry: its tag, type, count, and where it is (its 12 bytes, or 20 in BigTIFF).
pub(crate) struct Entry {
    pub(crate) tag: u16,
    pub(crate) kind: u16,
    count: u64,
    at: usize,
}

pub(crate) const TIFF_BYTE: u16 = 1;
pub(crate) const TIFF_ASCII: u16 = 2;
pub(crate) const TIFF_SHORT: u16 = 3;
pub(crate) const TIFF_LONG: u16 = 4;
pub(crate) const TIFF_UNDEFINED: u16 = 7;
pub(crate) const TIFF_LONG8: u16 = 16;
pub(crate) const TIFF_IFD8: u16 = 18;

impl<'a> Tiff<'a> {
    /// The byte order, then 42. A TIFF file (`bigtiff`) may be BigTIFF instead: 43, then 8 (the offset size) and 0.
    /// An EXIF block is always classic.
    pub(crate) fn new(data: &'a [u8], bigtiff: bool) -> Option<Tiff<'a>> {
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
    pub(crate) fn ifd0(&self) -> Option<usize> {
        self.offset_at(if self.big { 8 } else { 4 })
    }

    /// The entries of the IFD at `offset`, as far as they are in the block.
    pub(crate) fn entries(&self, offset: usize) -> impl Iterator<Item = Entry> + '_ {
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
    pub(crate) fn orientation_kind(&self, e: &Entry) -> bool {
        matches!(e.kind, TIFF_BYTE | TIFF_SHORT | TIFF_LONG) || (self.big && e.kind == TIFF_LONG8)
    }

    /// IFD0's orientation entries of a kind [`Tiff::orientation_kind`] takes, each with where its value bytes are
    /// in the block (an entry whose value is outside the block is left out). The reader takes the last readable
    /// one; the patcher sets the first value of each.
    pub(crate) fn orientation_entries(&self) -> impl Iterator<Item = (Range<usize>, Entry)> + '_ {
        self.ifd0()
            .into_iter()
            .flat_map(|ifd0| self.entries(ifd0))
            .filter(|e| e.tag == EXIF_TAG_ORIENTATION && self.orientation_kind(e))
            .filter_map(|e| Some((self.value_range(&e)?, e)))
    }

    /// An entry's first value as an unsigned integer (BYTE, SHORT or LONG; a LONG8 or IFD8 that fits a LONG).
    pub(crate) fn uint(&self, e: &Entry) -> Option<u32> {
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
    pub(crate) fn ascii(&self, e: &Entry) -> Option<&'a [u8]> {
        let v = self.value(e).filter(|_| e.kind == TIFF_ASCII)?;
        v.split(|&b| b == 0).next()
    }

    /// A BYTE or UNDEFINED entry's bytes.
    pub(crate) fn bytes(&self, e: &Entry) -> Option<&'a [u8]> {
        self.value(e).filter(|_| matches!(e.kind, TIFF_BYTE | TIFF_UNDEFINED))
    }
}
