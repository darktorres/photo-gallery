//! The file an input reads, through FFmpeg's custom I/O: the [`Source`] the callbacks read and seek, the name the
//! format probe is given, and the freeing of the context FFmpeg does not free.

use std::ffi::{CString, OsStr, c_int, c_void};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use super::generated::AVIOContext;
use super::load::Api;

/// The size of the buffer FFmpeg reads a file through (its own file protocol uses 32 KiB; a larger one saves calls).
pub(super) const IO_BUFFER: usize = 256 * 1024;
/// `AVSEEK_SIZE` and `AVSEEK_FORCE`: macros in the headers, which bindgen doesn't emit (as `NOPTS_VALUE`).
const AVSEEK_SIZE: c_int = 0x10000;
const AVSEEK_FORCE: c_int = 0x20000;
const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;
/// FFmpeg's error codes: `AVERROR_EOF` is `-FFERRTAG('E','O','F',' ')` (the tag's four bytes, little-endian, as one
/// number; a test checks it), and `AVERROR(EIO)` is the errno negated.
const AVERROR_EOF: c_int = -541_478_725;
const AVERROR_EIO: c_int = -5;

/// The file an input (`ffi::Input`) reads, through FFmpeg's I/O callbacks: its bytes as the file system has them, whatever the
/// file's name is. Owned by the `Input` (the callbacks point at it).
pub(super) struct Source {
    pub(super) file: std::fs::File,
    pub(super) len: u64,
}

/// FFmpeg's read callback: the next bytes of the file. Rust panics must not unwind into FFmpeg, so a panic is caught
/// and reported as an I/O error.
pub(super) unsafe extern "C" fn read_source(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `opaque` is the `Source` that `Input::open` gave this context, alive until the context is freed;
        // `buf` is `size` bytes of FFmpeg's buffer, valid for this call.
        let source = unsafe { &mut *opaque.cast::<Source>() };
        // SAFETY: FFmpeg gives `size` bytes at `buf` for this call (none if the size is negative).
        let out = unsafe { std::slice::from_raw_parts_mut(buf, usize::try_from(size).unwrap_or(0)) };
        match source.file.read(out) {
            Ok(0) => AVERROR_EOF,
            Ok(n) => c_int::try_from(n).unwrap_or(AVERROR_EIO),
            Err(_) => AVERROR_EIO,
        }
    }))
    .unwrap_or(AVERROR_EIO)
}

/// FFmpeg's seek callback. `AVSEEK_SIZE` asks for the file's length; `AVSEEK_FORCE` is a hint the file protocol takes
/// and this one doesn't need, so it is masked off.
pub(super) unsafe extern "C" fn seek_source(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `read_source`.
        let source = unsafe { &mut *opaque.cast::<Source>() };
        let whence = whence & !AVSEEK_FORCE;
        if whence == AVSEEK_SIZE {
            return i64::try_from(source.len).unwrap_or(-1);
        }
        let from = match whence {
            SEEK_SET => match u64::try_from(offset) {
                Ok(pos) => SeekFrom::Start(pos),
                Err(_) => return -1,
            },
            SEEK_CUR => SeekFrom::Current(offset),
            SEEK_END => SeekFrom::End(offset),
            _ => return -1,
        };
        source.file.seek(from).ok().and_then(|pos| i64::try_from(pos).ok()).unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

/// The name FFmpeg's format probe reads: `pg-video`, with the file's extension when it is ASCII. The name is never a
/// path (the file is read through the I/O), so the file's own name stays out of FFmpeg.
pub(super) fn hint_name(path: &Path) -> CString {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    let name = match ext {
        Some(ext) => format!("pg-video.{ext}"),
        None => "pg-video".to_string(),
    };
    CString::new(name).unwrap_or_default()
}

/// Frees a custom I/O context, its buffer and the source it read (FFmpeg doesn't free a custom context's I/O: it is
/// ours). FFmpeg may have replaced the buffer, and the context's field says which one it is.
pub(super) fn free_io(api: &Api, mut io: *mut AVIOContext, source: *mut Source) {
    // SAFETY: `io` is the context `open` made, freed once here; its buffer is the one FFmpeg holds in the field.
    unsafe {
        (api.av_free)((*io).buffer.cast());
        (api.avio_context_free)(&mut io);
        // SAFETY: `source` came from `Box::into_raw` in `open`, and no callback runs after the context is freed.
        drop(Box::from_raw(source));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_file_code_is_ffmpegs_tag() {
        assert_eq!(AVERROR_EOF, -c_int::from_le_bytes(*b"EOF "));
    }
}
