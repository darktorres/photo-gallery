//! The `--screenshot=<png>` test hook (with `--wait=<s>`, the most to wait for the window to settle, 10 by default):
//! save the window once it has settled (ready, no image loading, no QML timer pending, no animation, no library job or
//! write pending, no frame for a while), then quit (cpp/screenshot.cpp). C++ encodes the window as a PNG and calls
//! `screenshot_taken` here, which writes the image and the reason it was taken (`<png>.settle.txt`) at the path as the
//! system gave it, so a name that isn't UTF-8 is written under its own bytes. For checking the UI without a person at the
//! screen, with the QML's own hooks (TestHooks.qml).

use crate::screenshot_bridge::ffi;

use core::pin::Pin;
use std::io;
use std::path::Path;

use cxx_qt_lib::QQmlApplicationEngine;

use crate::command_line;

/// Schedule the screenshot if the command line asks for one.
pub(crate) fn schedule(engine: Pin<&mut QQmlApplicationEngine>) {
    if command_line::path("screenshot").is_none() {
        return;
    }
    let wait = command_line::hooks().wait;
    ffi::save_screenshot_later(engine, millis(wait));
}

/// The window has settled, or the wait ran out (on the GUI thread, from cpp/screenshot.cpp): write the image and its
/// log at the path the command line gave. `png` is empty when there is no window or the window couldn't be encoded;
/// `has_window` tells the two apart. The app quits after this.
#[expect(
    clippy::print_stderr,
    reason = "a screenshot run is a test run: its failures go to the console that started it"
)]
pub(crate) fn screenshot_taken(png: &[u8], settled: bool, has_window: bool, report: &str) {
    let Some(path) = command_line::path("screenshot") else { return };
    if !settled {
        eprintln!("screenshot: the window did not settle: {report}");
    }
    if let Err(e) = write_taken(path, png, has_window, report) {
        eprintln!("screenshot: can't save {}: {e}", path.display());
    }
}

/// Writes the settle log (`report` and a newline) beside `path`, then the image there. The log goes first, so it is
/// there even when the image can't be written. With no image, the error says why: there was no window, or the window
/// couldn't be encoded as a PNG.
fn write_taken(path: &Path, png: &[u8], has_window: bool, report: &str) -> io::Result<()> {
    let mut log = path.as_os_str().to_owned();
    log.push(".settle.txt");
    std::fs::write(log, format!("{report}\n"))?;
    if png.is_empty() {
        let reason = if has_window { "the window can't be encoded as a PNG" } else { "there is no window to save" };
        return Err(io::Error::other(reason));
    }
    std::fs::write(path, png)
}

/// Seconds as whole milliseconds; the conversion saturates at an int's range (NaN is 0).
fn millis(seconds: f64) -> i32 {
    pg_model::num::f64_to_i32(seconds * 1000.0)
}

#[cfg(test)]
mod tests {
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

    fn log_of(path: &Path) -> PathBuf {
        let mut log = path.as_os_str().to_owned();
        log.push(".settle.txt");
        PathBuf::from(log)
    }

    #[test]
    fn the_image_and_its_settle_log_are_written_at_the_exact_path() {
        let dir = Scratch::new("exact");
        let path = dir.0.join(pg_testkit::odd_name("shot"));
        write_taken(&path, b"\x89PNG", true, "settled after 5 ms").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        assert_eq!(std::fs::read_to_string(log_of(&path)).unwrap(), "settled after 5 ms\n");
    }

    #[test]
    fn a_window_that_cant_be_encoded_leaves_its_log_and_no_image() {
        let dir = Scratch::new("unencoded");
        let path = dir.0.join(pg_testkit::odd_name("shot"));
        let err = write_taken(&path, &[], true, "timed out after 10000 ms").unwrap_err();
        assert_eq!(err.to_string(), "the window can't be encoded as a PNG");
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(log_of(&path)).unwrap(), "timed out after 10000 ms\n");
    }

    #[test]
    fn no_window_and_an_unencoded_window_say_different_things() {
        let dir = Scratch::new("reasons");
        let no_window = write_taken(&dir.0.join("none.png"), &[], false, "timed out").unwrap_err().to_string();
        let unencoded = write_taken(&dir.0.join("bad.png"), &[], true, "timed out").unwrap_err().to_string();
        assert_eq!(no_window, "there is no window to save");
        assert_ne!(no_window, unencoded);
    }

    #[test]
    fn the_wait_is_whole_milliseconds_within_an_int() {
        assert_eq!(millis(10.0), 10_000);
        assert_eq!(millis(f64::NAN), 0);
        assert_eq!(millis(f64::INFINITY), i32::MAX);
    }
}
