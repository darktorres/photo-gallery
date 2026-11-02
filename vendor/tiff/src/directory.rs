use core::fmt;
use std::num::NonZeroU64;

use crate::{
    decoder::ifd::Entry,
    tags::{IfdPointer, Tag},
};

/// An Image File Directory (IFD).
///
/// A directory is a map of [`Tag`]s to [`Value`](crate::decoder::ifd::Value)s. The values are
/// stored anywhere in the file, with the directory containing the offsets and length of the
/// associated values for each tag present in the directory.
///
/// A directory can be created with with
/// [`Decoder::read_directory`](crate::decoder::Decoder::read_directory) or as an empty directory
/// to be extended with entries. A directory may be used with
/// [`Decoder::read_directory_tags`](crate::decoder::Decoder::read_directory_tags) to read the
/// values associated with tags from an underlying file.
#[doc(alias = "IFD")]
pub struct Directory {
    /// There are at most `u16::MAX` entries in any single directory, the count is stored as a
    /// 2-byte value. The order in the file is implied to be ascending by tag value (the decoder
    /// does not mind unordered entries).
    ///
    /// photo-gallery: sorted by tag, one entry per tag (the last given), so it holds what
    /// upstream's `BTreeMap<u16, Entry>` held, in its order; a vector can be kept from one
    /// directory to the next (`decoder::Scratch`), where a map's nodes are freed and made anew.
    pub(crate) entries: Vec<(u16, Entry)>,
    pub(crate) next_ifd: Option<NonZeroU64>,
}

impl Directory {
    /// Create a directory in an initial state without entries. Note that an empty directory can
    /// not be encoded in a file, it must contain at least one entry.
    pub fn empty() -> Self {
        Directory {
            entries: Vec::new(),
            next_ifd: None,
        }
    }

    /// Retrieve the value associated with a tag.
    pub fn get(&self, tag: Tag) -> Option<&Entry> {
        self.position(tag).ok().map(|i| &self.entries[i].1)
    }

    /// photo-gallery: where `tag` is in the sorted entries, or would be.
    fn position(&self, tag: Tag) -> Result<usize, usize> {
        let tag = tag.to_u16();
        self.entries.binary_search_by_key(&tag, |(t, _)| *t)
    }

    /// photo-gallery: sorts the entries by tag and keeps the last of each tag, which is what
    /// inserting them in turn into upstream's map left (a stable sort keeps the order they were
    /// given in among equal tags).
    pub(crate) fn normalize(&mut self) {
        self.entries.sort_by_key(|(t, _)| *t);
        self.entries.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                core::mem::swap(later, earlier);
                true
            } else {
                false
            }
        });
    }

    /// Check if the directory contains a specified tag.
    pub fn contains(&self, tag: Tag) -> bool {
        self.position(tag).is_ok()
    }

    /// Iterate over all known and unknown tags in this directory.
    pub fn iter(&self) -> impl Iterator<Item = (Tag, &Entry)> + '_ {
        self.entries
            .iter()
            .map(|(k, v)| (Tag::from_u16_exhaustive(*k), v))
    }

    /// Insert additional entries into the directory.
    ///
    /// Note that a directory can contain at most `u16::MAX` values. There may be one entry that
    /// does not fit into the directory. This entry is silently ignored (please check [`Self::len`]
    /// to detect the condition). Providing a tag multiple times or a tag that already exists
    /// within this directory overwrites the entry.
    pub fn extend(&mut self, iter: impl IntoIterator<Item = (Tag, Entry)>) {
        // Code size conscious extension, avoid monomorphic extensions with the assumption of these
        // not being performance sensitive in practice. (Maybe we have a polymorphic interface for
        // the crate usage in the future.
        self.extend_inner(iter.into_iter().by_ref())
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        // The keys are `u16`. Since IFDs are required to have at least one entry this would have
        // been a trivial thing to do in the specification by storing it minus one but alas. In
        // BigTIFF the count is stored as 8-bit anyways.
        self.entries.len()
    }

    /// Check if there are any entries in this directory. Note that an empty directory can not be
    /// encoded in the file, it must contain at least one entry.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the pointer to the next IFD, if it was defined.
    pub fn next(&self) -> Option<IfdPointer> {
        self.next_ifd.map(|n| IfdPointer(n.get()))
    }

    pub fn set_next(&mut self, next: Option<IfdPointer>) {
        self.next_ifd = next.and_then(|n| NonZeroU64::new(n.0));
    }

    fn extend_inner(&mut self, iter: &mut dyn Iterator<Item = (Tag, Entry)>) {
        // If the tag is already present, it will be overwritten.
        // photo-gallery: appended, then sorted with the last of each tag kept (`normalize`).
        for (tag, entry) in iter {
            self.entries.push((tag.to_u16(), entry));
        }
        self.normalize();
    }

    pub(crate) fn encoded_len<K: crate::encoder::TiffKind>(&self) -> u64 {
        let (len_count, len_offset);

        match std::mem::size_of::<K::OffsetType>() {
            offset @ 4 => {
                // non-BigTiff uses a `u16` here.
                len_count = 2;
                len_offset = offset as u64;
            }
            offset => {
                let offset = offset as u64;
                // assuming BigTIFF
                len_count = offset;
                len_offset = offset;
            }
        }

        // Each entry is 12 bytes in classic TIFF, 20 bytes in BigTIFF.
        let entry_bytes = 4 + 2 * len_offset;
        len_count + entry_bytes * self.entries.len() as u64 + len_offset
    }
}

impl fmt::Debug for Directory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Directory")
            .field(
                "entries",
                &self.entries.iter().map(|(k, v)| (Tag::from_u16(*k), v)),
            )
            .field("next_ifd", &self.next_ifd)
            .finish()
    }
}

impl core::iter::FromIterator<(Tag, Entry)> for Directory {
    fn from_iter<T: IntoIterator<Item = (Tag, Entry)>>(iter: T) -> Self {
        let mut dir = Directory::empty();
        dir.extend(iter);
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::Directory;
    use crate::{decoder::ifd::Entry, tags::Tag};

    #[test]
    fn directory_multiple_entries() {
        let mut dir = Directory::empty();
        assert_eq!(dir.len(), 0);

        dir.extend((0..=u16::MAX).map(|i| {
            let tag = Tag::Unknown(1);
            let entry = Entry::new_u64(crate::tags::Type::BYTE, i.into(), [0; 8]);
            (tag, entry)
        }));

        assert_eq!(dir.len(), 1, "Only one tag was ever modified");

        assert_eq!(
            dir.get(Tag::Unknown(1))
                .expect("tag 1 should be present after this chain")
                .count(),
            u16::MAX.into()
        );
    }

    #[test]
    fn iteration_order() {
        let mut dir = Directory::empty();
        assert_eq!(dir.len(), 0);

        let fake_entry = Entry::new_u64(crate::tags::Type::BYTE, 0, [0; 8]);
        dir.extend((0..32).map(|i| {
            let tag = Tag::Unknown(i);
            let entry = fake_entry.clone();
            (tag, entry)
        }));

        let iter_order: Vec<u16> = dir.iter().map(|(tag, _e)| tag.to_u16()).collect();
        assert_eq!(
            iter_order,
            (0..32).collect::<Vec<_>>(),
            "Tags must be in ascending order according to the specification"
        );
    }
}
