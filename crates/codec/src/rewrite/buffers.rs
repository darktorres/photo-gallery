//! The fallible buffers the rewrites allocate: a raster's bytes are decoded into them, and an operation's output is built in
//! them. A module of their own, so the operations and the rasters don't depend on each other for them.

use crate::{Error, Result};

/// An empty buffer with room for `len` bytes (a size [`super::raster::decoded_size`] passed), allocated fallibly: too little
/// memory is an error, not an abort.
pub(super) fn reserved(len: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).map_err(|e| Error::decode("not enough memory to decode the image", e))?;
    Ok(buf)
}
