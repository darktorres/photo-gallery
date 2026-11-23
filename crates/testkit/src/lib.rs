//! Helpers for the crates' tests. Tests write their files under `PG_TEST_DIR` when it is set, and under the temp folder
//! otherwise, in the folder each test names. Set `PG_TEST_DIR` to keep the files off the system drive.
//!
//! The folders and their protocol are in pg-platform (`testing`), and a test names them there (`pg_platform::testing`).
//! The tests' generator is `pg-testrng`. The image files a test writes are the codec's (`pg_codec::fixtures`: PNG, BMP, GIF, TIFF, APNG, JPEG and
//! WebP), and this crate keeps the raw RGB pictures they are written from (`rgb_pattern` and the others). The video
//! fixture is the clip the tests share (`VIDEO_SAMPLE`), kept in the video crate's tests folder and read here.

#![forbid(unsafe_code)]

pub mod inputs;

use std::fmt::Display;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What a test helper couldn't make: the step that failed, and its cause (the file system's).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A run folder of the test binary (see [`run_folder`]).
    #[error("make the run folder {prefix} in {scratch}")]
    RunFolder {
        /// The run folder's name prefix (the test's own name).
        prefix: String,
        /// The scratch folder the run folder was to be made in.
        scratch: String,
        /// The I/O error under the failure.
        #[source]
        source: io::Error,
    },
    /// A folder a test asked for under its run folder (see [`run_folder_with`]).
    #[error("make the folder {}", path.display())]
    Folder {
        /// The folder that couldn't be made.
        path: PathBuf,
        /// The I/O error under the failure.
        #[source]
        source: io::Error,
    },
}

/// The result of a test helper.
pub type Result<T> = std::result::Result<T, Error>;

/// The sample clip (tools/make_test_video.py): 64x48, six frames, H.264. The video crate's and the library's tests read
/// it from here.
pub const VIDEO_SAMPLE: &[u8] = include_bytes!("../../video/tests/sample.mp4");

/// A fresh folder for one test (or one part of a test) in a test binary's run: `prefix-<process id>-<n>` under the base
/// (`PG_TEST_DIR`, else the temp folder's `scratch`). The run folders of `prefix` that no running process holds are
/// removed first, and the folder is owned by its lock from the moment it has a name (see `pg_platform::testing`).
pub fn run_folder(scratch: &str, prefix: &str) -> Result<PathBuf> {
    pg_platform::testing::run_folder(scratch, prefix).map_err(|source| Error::RunFolder {
        prefix: prefix.to_string(),
        scratch: scratch.to_string(),
        source,
    })
}

/// [`run_folder`], with the folders `dirs` (paths relative to it, made as needed) made in it.
pub fn run_folder_with(scratch: &str, prefix: &str, dirs: &[&str]) -> Result<PathBuf> {
    let dir = run_folder(scratch, prefix)?;
    for sub in dirs {
        let sub = dir.join(sub);
        std::fs::create_dir_all(&sub).map_err(|source| Error::Folder { path: sub.clone(), source })?;
    }
    Ok(dir)
}

/// Held by each test that runs a detection or thumbnail pass: the pass's process-wide state (the detector's injected
/// fault, file ids that repeat from one catalog to the next) is shared by every test of the binary, so those tests run
/// one at a time. Each test binary has its own copy.
pub static ONE_PASS: Mutex<()> = Mutex::new(());

/// The interval a [`wait_until`] polls at.
const POLL: Duration = Duration::from_millis(20);

/// Polls `done` every 20 ms until it holds, for a step another thread or process makes. Once `within` has passed it
/// panics with `what()` (made only then) and the bound. Every wait in the tests goes through here, so the bound is
/// stated at each call and the interval is one.
pub fn wait_until<W: Display>(within: Duration, what: impl Fn() -> W, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < within, "waited {within:?} for {}", what());
        std::thread::sleep(POLL);
    }
}

/// [`wait_until`] for a file: until `path` exists.
pub fn wait_for_path(path: &Path, within: Duration) {
    wait_until(within, || format!("{} to appear", path.display()), || path.exists());
}

/// The interleaved RGB bytes of a `w` × `h` picture whose channels step across it, so a crop or a turn moves pixels that
/// differ from each other.
pub fn rgb_pattern(w: usize, h: usize) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            // Each channel is below 256 by its construction (`w`, `h` and the modulus), so the fallback never applies.
            [x * 255 / w, y * 255 / h, (x * 7 + y * 13) % 256].map(|v| u8::try_from(v).unwrap_or(u8::MAX))
        })
        .collect()
}

/// A channel value: `v` wrapped to a byte (the pictures' steps wrap past 255).
fn wrap(v: u32) -> u8 {
    u8::try_from(v % 256).unwrap_or(u8::MAX)
}

/// The interleaved RGB bytes of a `w` × `h` picture, each pixel from `pixel(x, y)`.
fn picture(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let pixel = &pixel;
    (0..h).flat_map(|y| (0..w).flat_map(move |x| pixel(x, y))).collect()
}

/// A textured `w` × `h` picture, the same bytes on every call: the duplicate pair's `a.png` and `b.png`.
pub fn textured_rgb(w: u32, h: u32) -> Vec<u8> {
    picture(w, h, |x, y| [wrap(x * x + y * 31), wrap(x * 7 + y * y), wrap((x ^ y) * 5)])
}

/// Another `w` × `h` picture, not a duplicate of [`textured_rgb`].
pub fn other_rgb(w: u32, h: u32) -> Vec<u8> {
    picture(w, h, |x, y| [wrap(y * y + 3 * x), wrap(x * 13 + 5), wrap(y * 3)])
}

/// A `w` × `h` gradient: red and green step across it, blue is 128.
pub fn gradient_rgb(w: u32, h: u32) -> Vec<u8> {
    picture(w, h, |x, y| [wrap(x * 4), wrap(y * 5), 128])
}

/// A `w` × `h` letterboxed picture: black bars `bar` deep at the top and bottom, photo-like content (blocks of hashed
/// colours with fine noise) between them.
pub fn letterboxed_rgb(w: u32, h: u32, bar: u32) -> Vec<u8> {
    let hash = |a: u32, b: u32| (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
    picture(w, h, |x, y| {
        if !(bar..h - bar).contains(&y) {
            return [0, 0, 0];
        }
        let block = hash(x / 6, y / 6);
        let noise = hash(x, y) % 24;
        // 60 to 232, so it fits a channel.
        let c = |shift: u32| u8::try_from(60 + (block >> shift) % 150 + noise).unwrap_or(u8::MAX);
        [c(0), c(8), c(16)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_folder_is_fresh_and_each_call_has_its_own() {
        let first = run_folder("scratch_testkit", "fresh").unwrap();
        std::fs::write(first.join("kept.txt"), b"x").unwrap();
        let second = run_folder("scratch_testkit", "fresh").unwrap();
        assert_ne!(first, second);
        assert!(first.join("kept.txt").is_file(), "a call does not remove another call's folder");
        assert_eq!(std::fs::read_dir(&second).unwrap().count(), 0, "a new folder is empty");
    }
}
