//! Where a pipeline's readers get their files' bytes: the seam between the pipeline and the disk. The pipeline opens a
//! file through a [`FileSource`], learns its size (the pool's admission is sized by it), and then reads it, whole or by a
//! gather, through the [`ReadSeek`] the source handed out. The default, [`OsFiles`], is the system's files, opened as
//! the pipeline always opened them; a test hands in a source of its own (files that read slowly, fail or stall), so the
//! reader gate is tested without real files.

use std::io;

use pg_platform::native::NativePath;

/// A file the pipeline reads: its bytes, from its start, with seeks for a gather. Implemented for anything that is
/// both, so a `std::fs::File` and an in-memory cursor are one.
pub trait ReadSeek: io::Read + io::Seek {}

impl<T: io::Read + io::Seek + ?Sized> ReadSeek for T {}

/// How the file will be read, so the source can open it for that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Whole, from its start to its end: the system may read ahead.
    Sequential,
    /// Here and there (a gather): the system reads no further ahead than each read asks.
    Random,
}

/// An opened file: its reader, and its size in bytes (what the pool admits it by).
pub struct Opened {
    /// The file's bytes.
    pub file: Box<dyn ReadSeek + Send>,
    /// Its size.
    pub size: usize,
}

/// Opens the files a pipeline reads. One virtual call per file.
pub trait FileSource: Send + Sync {
    /// The file at `path`, opened for `access`, with its size. An error is the item's read error.
    fn open(&self, path: &NativePath, access: Access) -> io::Result<Opened>;
}

/// The system's files: `pg_platform::file::open_read` for a whole read, `open_read_random` for a gather, and the size
/// from the open handle's metadata.
pub struct OsFiles;

impl FileSource for OsFiles {
    fn open(&self, path: &NativePath, access: Access) -> io::Result<Opened> {
        let file = match access {
            Access::Sequential => pg_platform::file::open_read(path)?,
            Access::Random => pg_platform::file::open_read_random(path)?,
        };
        let size =
            usize::try_from(file.metadata()?.len()).map_err(|e| io::Error::new(io::ErrorKind::FileTooLarge, e))?;
        Ok(Opened { file: Box::new(file), size })
    }
}
