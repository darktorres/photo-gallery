//! Windows paths handed outside the standard library. `std::fs` already turns a long path into its verbatim form
//! (`\\?\`) itself, so a path for std is used as it is. FFmpeg and direct Win32 calls (the scanner's
//! `CreateFileW`) don't: a path past the 260-character limit reaches them in its verbatim form, made here.
//! ExifTool takes the plain path, as a `\\?\` path isn't one it reads (its own long-name support is a separate
//! option, FINDINGS "I/O").

use std::borrow::Cow;

/// Shorter than this, a path (a folder, or a file in one) is used as it is.
const SHORT: usize = 240;

/// The verbatim form of an absolute path (`D:\…` or `\\server\share\…`) when it is long, else the path itself.
pub fn verbatim(path: &str) -> Cow<'_, str> {
    if path.len() < SHORT || path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        Cow::Borrowed(path)
    } else if let Some(unc) = path.strip_prefix(r"\\") {
        Cow::Owned(format!(r"\\?\UNC\{unc}"))
    } else {
        Cow::Owned(format!(r"\\?\{path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_are_verbatim_unc_too() {
        assert_eq!(verbatim(r"D:\a\b.jpg"), r"D:\a\b.jpg");
        let dir = "x".repeat(250);
        assert_eq!(verbatim(&format!(r"D:\{dir}\b.jpg")), format!(r"\\?\D:\{dir}\b.jpg"));
        assert_eq!(verbatim(&format!(r"\\nas\photos\{dir}\b.jpg")), format!(r"\\?\UNC\nas\photos\{dir}\b.jpg"));
        let already = format!(r"\\?\D:\{dir}");
        assert_eq!(verbatim(&already), already);
    }

    /// std reads a file through a path past the limit as it is (it makes the verbatim form itself).
    #[test]
    fn std_takes_long_paths_as_they_are() {
        let root = std::env::temp_dir().join(format!("pg-paths-{}", std::process::id()));
        let mut dir = root.clone();
        while dir.as_os_str().len() < 300 {
            dir.push("a-folder-name-of-forty-characters-long--");
        }
        let file = dir.join("f.bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, b"long").unwrap();
        assert!(file.as_os_str().len() > 300);
        assert_eq!(std::fs::read(&file).unwrap(), b"long");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
