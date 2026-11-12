//! Helpers for the crates' tests. Tests write their files under `PG_TEST_DIR` when it is set, and under the temp folder
//! otherwise, in the folder each test names. Set `PG_TEST_DIR` to keep the files off the system drive.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

pub mod held;

/// `text` followed by a unit that is no UTF-8, as a name the system can hold: a lone surrogate (`0xD800`) on Windows, a
/// raw `0xFF` byte on Unix. For the tests that a name, a path or an argument is kept exactly.
pub fn odd_name(text: &str) -> OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = text.encode_utf16().collect();
        units.push(0xD800);
        OsString::from_wide(&units)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0xFF);
        OsString::from_vec(bytes)
    }
}

/// A test needs an external tool that is not installed: the test returns after this call. Without `PG_REQUIRE_TOOLS`
/// it prints that it skipped and passes; with it (CI sets it) the test fails, so a missing tool can't pass silently.
#[expect(clippy::print_stderr, reason = "a skipped test says so on stderr, where the test runner prints it")]
pub fn missing_tool(tool: &str) {
    assert!(std::env::var_os("PG_REQUIRE_TOOLS").is_none(), "{tool} not found, and PG_REQUIRE_TOOLS is set");
    eprintln!("{tool} not found: skipped");
}

/// A test that can't check what it is for in this environment (the reason says what) returns after this call: it
/// passes, with the reason on stderr where the test runner prints it.
#[expect(clippy::print_stderr, reason = "a skipped test says so on stderr, where the test runner prints it")]
pub fn skip(reason: &str) {
    eprintln!("skipped: {reason}");
}

/// The base folder for a test's files: `PG_TEST_DIR` when it is set, else the temp folder's `scratch` folder. The name
/// is a plain folder name (`"scratch_rate"`): no test writes to a drive it was not told about.
fn base(scratch: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("PG_TEST_DIR") {
        // `.env` writes paths with forward slashes, and the verbatim (`\\?\`) paths that pg-platform opens are not
        // normalized: a test's folders need the native separator, as the paths the app itself is given have.
        return native_separators(&dir);
    }
    std::env::temp_dir().join(scratch)
}

/// `dir` with its forward slashes made the native separator. On Windows this is done on the UTF-16 units, so every other
/// unit is kept, a lone surrogate included. On Unix a forward slash is already the separator: the path is kept as it is.
#[cfg(windows)]
fn native_separators(dir: &OsStr) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> =
        dir.encode_wide().map(|unit| if unit == u16::from(b'/') { u16::from(b'\\') } else { unit }).collect();
    PathBuf::from(std::ffi::OsString::from_wide(&units))
}

/// `dir` as it is: on Unix a forward slash is the separator, and every other byte is kept.
#[cfg(not(windows))]
fn native_separators(dir: &OsStr) -> PathBuf {
    PathBuf::from(dir)
}

/// A fresh folder for one test: `name-<process id>` under the base, emptied if it exists. Fails if it can't be made.
pub fn test_folder(scratch: &str, name: &str) -> Result<PathBuf> {
    let dir = base(scratch).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("make the test folder {}", dir.display()))?;
    Ok(dir)
}

/// The name of the file each run folder holds, locked by the process that made the folder until the process ends.
const OWNER: &str = ".owner";

/// The name the owner file is first made under, locked, and renamed to [`OWNER`] once it is. Each run folder is made
/// exclusively, so this name is its own.
const OWNER_TMP: &str = ".owner.tmp";

/// How long a run folder without an owner file is left alone. The owner file is made just after its folder, so a folder
/// this young is being made, not a leftover.
const GRACE: Duration = Duration::from_secs(60);

/// The owner files this process holds locked. They stay open until the process ends, so no run takes their folders for
/// leftovers.
static HELD: Mutex<Vec<File>> = Mutex::new(Vec::new());

/// Numbers the run folders this process makes, so that every call gets a name of its own.
static RUN_FOLDERS: AtomicU64 = AtomicU64::new(0);

/// Whether a run folder is in use. Its owner file locked, by this process or another, means it is. A folder with no owner
/// file is in use only while it is young (see [`GRACE`]): an earlier build's folders have none, and are leftovers. If the
/// owner file can't be tested, the folder is kept.
fn in_use(folder: &Path) -> bool {
    let file = match OpenOptions::new().read(true).write(true).open(folder.join(OWNER)) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return young(folder),
        Err(_) => return true,
    };
    match file.try_lock() {
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => true,
    }
}

/// Whether a folder was made within [`GRACE`]. A time it can't be read, or that lies in the future, counts as young.
fn young(folder: &Path) -> bool {
    match folder.metadata().and_then(|meta| meta.modified()) {
        Ok(modified) => modified.elapsed().is_ok_and(|age| age < GRACE),
        Err(_) => true,
    }
}

/// A fresh folder for one test (or one part of a test) in a test binary's run: `prefix-<process id>-<n>` under the base,
/// where `n` counts this process's calls, so the tests that run at once in one process each have their own. The run
/// folders of `prefix` that no running process holds are removed first (the ones earlier runs left, and any that have
/// ended). A folder another running process holds is kept, so two test binaries can share a base. Fails if it can't be
/// made.
pub fn run_folder(scratch: &str, prefix: &str) -> Result<PathBuf> {
    let base = base(scratch);
    let earlier = format!("{prefix}-");
    for old in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let name = old.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&earlier) else { continue };
        // Exactly `<id>-<n>` (or an earlier build's `<id>`), digits only: a folder of another prefix that ends in digits
        // (`run-5` with prefix `run`) has more segments, so it is not taken for a run folder.
        let parts: Vec<&str> = rest.split('-').collect();
        let numbers = (1..=2).contains(&parts.len())
            && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
        if numbers && !in_use(&old.path()) {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    // The name is claimed by creating the folder, which fails if the name is taken: by another run in this process, or by
    // another process with the same id (a process id is only unique within one container, so two containers sharing a
    // base can both have process 18). The next number is tried then. Nothing is removed to make room. The base itself is
    // made first: `create_dir` makes no parents, and a fresh machine (a new container's `/tmp`) has none.
    std::fs::create_dir_all(&base).with_context(|| format!("make the scratch folder {}", base.display()))?;
    let pid = std::process::id();
    let dir = loop {
        let n = RUN_FOLDERS.fetch_add(1, Ordering::Relaxed);
        let candidate = base.join(format!("{earlier}{pid}-{n}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("make the run folder {}", candidate.display()));
            }
        }
    };
    // The owner file is locked before it takes its name. A cleanup that runs in the gap (in this process or another) sees
    // an unlocked OWNER as a leftover and would remove the folder; a folder with no OWNER yet is kept while it is young.
    let tmp = dir.join(OWNER_TMP);
    let owner = File::create(&tmp).with_context(|| format!("make the owner file of {}", dir.display()))?;
    owner.lock().with_context(|| format!("lock the owner file of {}", dir.display()))?;
    std::fs::rename(&tmp, dir.join(OWNER)).with_context(|| format!("name the owner file of {}", dir.display()))?;
    HELD.lock().unwrap_or_else(PoisonError::into_inner).push(owner);
    Ok(dir)
}

/// [`run_folder`], with the folders `dirs` (paths relative to it, made as needed) made in it.
pub fn run_folder_with(scratch: &str, prefix: &str, dirs: &[&str]) -> Result<PathBuf> {
    let dir = run_folder(scratch, prefix)?;
    for sub in dirs {
        let sub = dir.join(sub);
        std::fs::create_dir_all(&sub).with_context(|| format!("make the folder {}", sub.display()))?;
    }
    Ok(dir)
}

/// An animated GIF of `frames` 4 × 4 frames, the first red and each after it brighter. The red level steps by 60, so
/// at most 5 frames (0 to 240): more is an error.
pub fn gif(frames: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
    for i in 0..frames {
        let red = i
            .checked_mul(60)
            .and_then(|level| u8::try_from(level).ok())
            .ok_or_else(|| anyhow!("frame {i} of {frames}: a test GIF has at most 5 frames"))?;
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([red, 0, 0, 255]));
        enc.encode_frame(image::Frame::new(img)).with_context(|| format!("encode GIF frame {i}"))?;
    }
    drop(enc);
    Ok(out)
}

/// `rgba` (`w` × `h` pixels, four bytes each) as a PNG, with its transparency.
pub fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| anyhow!("{} bytes are not {w} × {h} RGBA pixels", rgba.len()))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, image::ImageFormat::Png).context("encode the PNG")?;
    Ok(file.into_inner())
}

/// `rgb` (`w` × `h` pixels, three bytes each) as a BMP.
pub fn bmp_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Bmp)
}

/// `rgb` as a GIF, one frame.
pub fn gif_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Gif)
}

/// `rgb` as a PNG.
pub fn png_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Png)
}

/// `rgb` as a classic TIFF (`image` writes no BigTIFF: the codec's fixtures do).
pub fn tiff_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Tiff)
}

fn still(w: u32, h: u32, rgb: &[u8], format: image::ImageFormat) -> Result<Vec<u8>> {
    let img = image::RgbImage::from_raw(w, h, rgb.to_vec())
        .ok_or_else(|| anyhow!("{} bytes are not {w} × {h} RGB pixels", rgb.len()))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, format).with_context(|| format!("encode the {format:?}"))?;
    Ok(file.into_inner())
}

/// An animated PNG of `frames` 2 × 2 frames of one grey, or a still PNG for one frame.
pub fn apng(frames: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 2);
    enc.set_color(png::ColorType::Rgb);
    if frames > 1 {
        enc.set_animated(frames, 0).context("make the PNG animated")?;
    }
    let mut w = enc.write_header().context("write the PNG header")?;
    for _ in 0..frames {
        w.write_image_data(&[7; 12]).context("write a PNG frame")?;
    }
    w.finish().context("finish the PNG")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_folder_is_fresh_and_a_run_folder_replaces_earlier_runs() {
        let scratch = "pg-testkit-check";
        // Unless PG_TEST_DIR is set, which the default covers: then the folders are made under it instead.
        let first = test_folder(scratch, "fresh").unwrap();
        std::fs::write(first.join("left-over.txt"), b"x").unwrap();
        let again = test_folder(scratch, "fresh").unwrap();
        assert_eq!(first, again);
        assert_eq!(std::fs::read_dir(&again).unwrap().count(), 0, "emptied");

        // An earlier run's folder, whose owner file is no longer locked: the run has ended.
        let earlier = base(scratch).join("run-999999999");
        std::fs::create_dir_all(&earlier).unwrap();
        File::create(earlier.join(OWNER)).unwrap();
        let run = run_folder(scratch, "run").unwrap();
        assert!(run.is_dir());
        assert!(!earlier.exists(), "an earlier run's folder is removed");
    }

    #[test]
    fn a_folder_without_an_owner_file_is_kept_while_young() {
        let scratch = "pg-testkit-check-young";
        // Made just now: another process may be between making the folder and its owner file, so it is kept.
        let young = base(scratch).join("grace-999999999-0");
        // Made fresh: a folder left by an earlier run of this test is older than the grace period.
        let _ = std::fs::remove_dir_all(&young);
        std::fs::create_dir_all(&young).unwrap();
        let _ = run_folder(scratch, "grace").unwrap();
        assert!(young.is_dir(), "a folder made just now is not removed");
    }

    #[test]
    fn a_later_run_folder_leaves_this_process_s_earlier_one_alone() {
        let scratch = "pg-testkit-check-keep";
        let first = run_folder(scratch, "shared").unwrap();
        std::fs::write(first.join("kept.txt"), b"x").unwrap();
        let second = run_folder(scratch, "shared").unwrap();
        assert_ne!(first, second);
        assert!(first.join("kept.txt").is_file(), "a call in this process does not remove another test's folder");
        // A prefix that starts the same way is another test's: `shared-roots` is not `shared`'s run folder.
        let other = base(scratch).join("shared-roots-7");
        std::fs::create_dir_all(&other).unwrap();
        // A prefix `shared-5` whose folders end in digits: `shared-5-<id>-<n>` has one segment more than a run folder.
        let longer = base(scratch).join("shared-5-4242-0");
        std::fs::create_dir_all(&longer).unwrap();
        let _ = run_folder(scratch, "shared").unwrap();
        assert!(other.is_dir(), "a folder of another prefix is not removed");
        assert!(longer.is_dir(), "a folder of a prefix that ends in digits is not removed");
    }

    #[test]
    fn a_run_folder_whose_owner_still_holds_it_is_kept() {
        let scratch = "pg-testkit-check-owner";
        // Another process's run folder, held the way a running process holds its own.
        let held = base(scratch).join("owned-999999999-0");
        std::fs::create_dir_all(&held).unwrap();
        let owner = File::create(held.join(OWNER)).unwrap();
        owner.lock().unwrap();
        let _ = run_folder(scratch, "owned").unwrap();
        assert!(held.join(OWNER).is_file(), "a folder its owner holds is not removed");
        drop(owner);
        let _ = run_folder(scratch, "owned").unwrap();
        assert!(!held.exists(), "once its owner has ended, its folder is removed");
    }

    #[test]
    fn run_folders_made_at_once_in_one_process_are_distinct() {
        let scratch = "pg-testkit-check-threads";
        let folders: Vec<PathBuf> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..16).map(|_| s.spawn(|| run_folder(scratch, "threads").unwrap())).collect();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect()
        });
        let mut unique = folders.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), folders.len(), "every call has a folder of its own");
        for folder in &folders {
            assert!(folder.is_dir(), "{} survives the other calls", folder.display());
        }
    }
}
