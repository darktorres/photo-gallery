//! The `--screenshot=<png>` test hook (with `--wait=<s>`, the most to wait for the window to settle, 10 by default):
//! save the window once it has settled (ready, no image loading, no QML timer pending, no animation, no library job or
//! write pending, no frame for a while), then quit (cpp/screenshot.cpp). C++ encodes the window as a PNG and calls
//! `screenshot_taken` here, which writes the image and the reason it was taken (`<png>.settle.txt`) at the path as the
//! system gave it, so a name that isn't UTF-8 is written under its own bytes. For checking the UI without a person at the
//! screen, with the QML's own hooks (Main.qml).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("pg-desktop/cpp/screenshot.h");
        /// Once the window the engine shows has settled (at most `max_ms` after this call), hand it to
        /// `screenshot_taken`, then quit.
        #[namespace = "pg"]
        fn save_screenshot_later(engine: Pin<&mut QQmlApplicationEngine>, max_ms: i32);
    }

    extern "Rust" {
        #[namespace = "pg"]
        fn screenshot_taken(png: &[u8], settled: bool, report: &str);
    }
}

use core::pin::Pin;
use std::io;
use std::path::Path;

use cxx_qt_lib::QQmlApplicationEngine;

use crate::command_line;

/// Schedule the screenshot if the command line asks for one.
pub fn schedule(engine: Pin<&mut QQmlApplicationEngine>) {
    if command_line::path("screenshot").is_none() {
        return;
    }
    let wait: f64 = command_line::value("wait").and_then(|w| w.trim().parse().ok()).unwrap_or(10.0);
    ffi::save_screenshot_later(engine, millis(wait));
}

/// The window has settled, or the wait ran out (on the GUI thread, from cpp/screenshot.cpp): write the image and its
/// log at the path the command line gave. `png` is empty when the window couldn't be encoded. The app quits after this.
#[expect(
    clippy::print_stderr,
    reason = "a screenshot run is a test run: its failures go to the console that started it"
)]
fn screenshot_taken(png: &[u8], settled: bool, report: &str) {
    let Some(path) = command_line::path("screenshot") else { return };
    if !settled {
        eprintln!("screenshot: the window did not settle: {report}");
    }
    if let Err(e) = write_taken(path, png, report) {
        eprintln!("screenshot: can't save {}: {e}", path.display());
    }
}

/// Writes the settle log (`report` and a newline) beside `path`, then the image there. The log goes first, so it is
/// there even when the image can't be written.
fn write_taken(path: &Path, png: &[u8], report: &str) -> io::Result<()> {
    let mut log = path.as_os_str().to_owned();
    log.push(".settle.txt");
    std::fs::write(log, format!("{report}\n"))?;
    if png.is_empty() {
        return Err(io::Error::other("the window can't be encoded as a PNG"));
    }
    std::fs::write(path, png)
}

/// Seconds as whole milliseconds, within an int's range (NaN is 0).
fn millis(seconds: f64) -> i32 {
    pg_model::num::f64_to_i32((seconds * 1000.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;

    /// A scratch folder, removed when it goes.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("pg-screenshot-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A file name that is not UTF-8: a lone surrogate on Windows, a raw byte on Unix.
    #[cfg(windows)]
    fn odd_name() -> OsString {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = "shot".encode_utf16().collect();
        units.push(0xD800);
        OsString::from_wide(&units)
    }

    /// A file name that is not UTF-8: a lone surrogate on Windows, a raw byte on Unix.
    #[cfg(unix)]
    fn odd_name() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"shot\xFF".to_vec())
    }

    fn log_of(path: &Path) -> PathBuf {
        let mut log = path.as_os_str().to_owned();
        log.push(".settle.txt");
        PathBuf::from(log)
    }

    #[test]
    fn the_image_and_its_settle_log_are_written_at_the_exact_path() {
        let dir = Scratch::new("exact");
        let path = dir.0.join(odd_name());
        write_taken(&path, b"\x89PNG", "settled after 5 ms").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        assert_eq!(std::fs::read_to_string(log_of(&path)).unwrap(), "settled after 5 ms\n");
    }

    #[test]
    fn a_window_that_cant_be_encoded_leaves_its_log_and_no_image() {
        let dir = Scratch::new("unencoded");
        let path = dir.0.join(odd_name());
        assert!(write_taken(&path, &[], "timed out after 10000 ms").is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(log_of(&path)).unwrap(), "timed out after 10000 ms\n");
    }

    #[test]
    fn the_wait_is_whole_milliseconds_within_an_int() {
        assert_eq!(millis(10.0), 10_000);
        assert_eq!(millis(f64::NAN), 0);
        assert_eq!(millis(f64::INFINITY), i32::MAX);
    }
}
