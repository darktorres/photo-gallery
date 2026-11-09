//! Shared libraries: the file names of a library's versions, where a program's own libraries are found, and how they are
//! opened. Windows loads a DLL from a folder the program chose, and from the system's and the search's folders only (never
//! from PATH); elsewhere the system's own rules find a shared library by its name.

use std::path::{Path, PathBuf};

use libloading::Library;

/// The file name of version `major` of the library `stem` (FFmpeg's libraries, for one): `stem-major.dll` on Windows,
/// `libstem.major.dylib` on macOS, `libstem.so.major` elsewhere.
pub fn file_name(stem: &str, major: u32) -> String {
    if cfg!(windows) {
        format!("{stem}-{major}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.{major}.dylib")
    } else {
        format!("lib{stem}.so.{major}")
    }
}

/// Where the library file `file` is, from the folders `candidates` (in order): on Windows, the first candidate that holds it;
/// elsewhere, the name itself, for the system's own search. The error on Windows lists the candidates.
pub fn resolve(file: &str, candidates: &[PathBuf]) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Ok(PathBuf::from(file));
    }
    if let Some(path) = candidates.iter().map(|dir| dir.join(file)).find(|path| path.is_file()) {
        return Ok(path);
    }
    let listed: Vec<String> = candidates.iter().map(|dir| dir.display().to_string()).collect();
    Err(format!("{file} isn't in any of: {}", listed.join(", ")))
}

/// Opens the library at `path`: a file that `resolve` found, or the bare name for the system's search. On Windows the search
/// is limited to the DLL's own folder, the system's folders and the ones added to the search.
///
/// # Safety
///
/// Opening a library runs its initialization code, so the library must be one whose initialization is sound to run.
pub unsafe fn load(path: &Path) -> Result<Library, libloading::Error> {
    #[cfg(windows)]
    {
        use libloading::os::windows::{
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, Library as WinLibrary,
        };
        // SAFETY: the caller's contract; the flags search only the DLL's folder, the system's and the added ones (not PATH).
        unsafe {
            WinLibrary::load_with_flags(path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)
        }
        .map(Library::from)
    }
    #[cfg(not(windows))]
    {
        // SAFETY: the caller's contract.
        unsafe { Library::new(path) }
    }
}
