//! The bytes of file buffers a pass holds at most (`pg_jobs::pipeline::Options::buffer_bytes`): never zero, and made only
//! through conversions that are checked, so a caller cannot build a pool with no room or one whose size wrapped.

use core::num::NonZeroU64;

/// A pass's buffer budget in bytes: at least one. A target, not a strict bound: a file larger than it is read alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BufferBytes(NonZeroU64);

impl BufferBytes {
    /// `bytes`, or `None` for zero.
    pub const fn new(bytes: u64) -> Option<BufferBytes> {
        match NonZeroU64::new(bytes) {
            Some(bytes) => Some(BufferBytes(bytes)),
            None => None,
        }
    }

    /// `each` bytes for each of `count` (a read in flight, a worker), or `None` when that is zero or overflows `u64`.
    pub fn per(count: usize, each: u64) -> Option<BufferBytes> {
        BufferBytes::new(u64::try_from(count).ok()?.checked_mul(each)?)
    }

    /// `mib` mebibytes, or `None` when that is zero or overflows `u64`.
    pub fn from_mib(mib: u64) -> Option<BufferBytes> {
        BufferBytes::new(mib.checked_mul(1 << 20)?)
    }

    /// The bytes.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bytes_are_refused() {
        assert_eq!(BufferBytes::new(0), None);
        assert_eq!(BufferBytes::from_mib(0), None);
        assert_eq!(BufferBytes::per(0, 12 << 20), None);
        assert_eq!(BufferBytes::per(4, 0), None);
        assert_eq!(BufferBytes::new(1).map(BufferBytes::get), Some(1));
    }

    #[test]
    fn a_conversion_that_overflows_is_refused() {
        assert_eq!(BufferBytes::from_mib(u64::MAX >> 19), None);
        assert_eq!(BufferBytes::from_mib(u64::MAX >> 20).map(BufferBytes::get), Some((u64::MAX >> 20) << 20));
        assert_eq!(BufferBytes::per(usize::MAX, 1 << 40), None);
        assert_eq!(BufferBytes::per(20, 12 << 20).map(BufferBytes::get), Some(240 << 20));
    }
}
