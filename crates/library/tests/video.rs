//! Videos through FFmpeg: probing (size, rotation, duration, codec, date), the frame at a time, and a scan,
//! import and thumbnail pass over a folder of them. Also the viewer's animations (`pg_codec::animation`): WebP, GIF
//! and APNG made by FFmpeg, frame by frame. The clips are made with the `ffmpeg` command line (see [`ffmpeg`]; the test
//! is skipped without it), in a scratch folder (`PG_TEST_DIR`, else the temp folder's `scratch_video`).
//!
//! Every clip is 160 × 80 at 25 fps for 3 s: red on the left half and blue on the right for the first second, then
//! all green.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use pg_analysis::analyze;
use pg_catalog::Catalog;
use pg_catalog::fixture::state;
use pg_codec::animation;
use pg_codec::pixels::Rgb;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::analysis::Taken;
use pg_model::date::wall_clock_seconds;
use pg_model::rules::PathRules;
use pg_model::turns::Turns;
use pg_jobs_testkit::Quiet;
use pg_thumbcache::ThumbCache;
use pg_video::{frame, poster_ms, probe};

const SCENE: &str = "color=c=red:s=80x80:r=25:d=3[a];color=c=blue:s=80x80:r=25:d=3[b];[a][b]hstack[ab];\
                     color=c=green:s=160x80:r=25:d=3[g];[ab][g]overlay=enable='gte(t,1)',format=yuv420p";

/// The ffmpeg program, if it runs: `PG_FFMPEG`; else the pinned build `just setup` fetches (`.deps/ffmpeg/bin`); else
/// `ffmpeg` on PATH.
fn ffmpeg() -> Option<PathBuf> {
    let exe = std::env::var_os("PG_FFMPEG").map(PathBuf::from).unwrap_or_else(|| {
        let pinned = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.deps/ffmpeg/bin/ffmpeg.exe");
        if pinned.is_file() { pinned } else { PathBuf::from("ffmpeg") }
    });
    Command::new(&exe).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| exe)
}

/// Runs ffmpeg with `args`: an error if it can't start or fails.
fn run(exe: &Path, args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new(exe).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output()?;
    anyhow::ensure!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    Ok(())
}

/// The clips, made once per run.
fn clips(exe: &Path) -> anyhow::Result<PathBuf> {
    let dir = pg_testkit::run_folder("scratch_video", "video")?;
    let media = dir.join("media");
    std::fs::create_dir_all(&media).with_context(|| format!("make the media folder {}", media.display()))?;
    let out = |name: &str| media.join(name).to_string_lossy().into_owned();
    let encode = |name: &str, codec: &[&str]| {
        let mut args = vec!["-f", "lavfi", "-i", SCENE];
        args.extend_from_slice(codec);
        let path = out(name);
        args.push(&path);
        run(exe, &args)
    };
    encode("h264.mp4", &["-c:v", "libx264", "-metadata", "creation_time=2022-03-04T05:06:07Z"])?;
    encode("vp9.webm", &["-c:v", "libvpx-vp9", "-b:v", "200k"])?;
    encode("av1.mp4", &["-c:v", "libaom-av1", "-cpu-used", "8", "-b:v", "200k"])?;
    encode("prores.mov", &["-c:v", "prores_ks"])?;
    // A phone held upright: stored landscape, displayed turned 90° counterclockwise.
    run(exe, &["-display_rotation", "90", "-i", &out("h264.mp4"), "-c", "copy", &out("rotated.mp4")])?;
    // A selfie camera's mirror: displayed flipped left to right.
    run(exe, &["-display_hflip", "-i", &out("h264.mp4"), "-c", "copy", &out("mirrored.mp4")])?;
    // Animations: 0.8 to 1.2 s (red and blue, then green), outside the media folder.
    let anim = dir.join("anim");
    std::fs::create_dir_all(&anim).with_context(|| format!("make the animation folder {}", anim.display()))?;
    for (name, codec) in [
        ("anim.webp", &["-c:v", "libwebp_anim", "-lossless", "1", "-loop", "0"][..]),
        ("anim.gif", &["-f", "gif"]),
        ("anim.png", &["-f", "apng", "-plays", "0"]),
    ] {
        let path = anim.join(name).to_string_lossy().into_owned();
        let mut args = vec!["-f", "lavfi", "-i", SCENE, "-ss", "0.8", "-t", "0.4"];
        args.extend_from_slice(codec);
        args.push(&path);
        run(exe, &args)?;
    }
    Ok(dir)
}

/// The pixel at (`x`, `y`): an error outside the frame.
fn pixel_at(img: &Rgb, x: usize, y: usize) -> anyhow::Result<[u8; 3]> {
    let index = if x < img.width { y.checked_mul(img.width).and_then(|row| row.checked_add(x)) } else { None };
    index
        .and_then(|i| img.pixels.as_chunks::<3>().0.get(i))
        .copied()
        .ok_or_else(|| anyhow!("({x}, {y}) is outside the {} × {} frame", img.width, img.height))
}

fn near(p: [u8; 3], want: [u8; 3]) -> bool {
    p.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 40)
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 128, 0];

#[test]
fn videos() {
    let Some(exe) = ffmpeg() else {
        pg_testkit::missing_tool("ffmpeg");
        return;
    };
    let dir = clips(&exe).unwrap();
    let media = dir.join("media");
    let pixel = |img: &Rgb, x: usize, y: usize| pixel_at(img, x, y).unwrap();

    // Each pass gives the frames whole, with their delays, red and blue first and green last (encoders may merge
    // identical frames into a longer one); a second pass (a loop) gives them again.
    for name in ["anim.webp", "anim.gif", "anim.png"] {
        let bytes: std::sync::Arc<[u8]> = std::fs::read(dir.join("anim").join(name)).unwrap().into();
        for _ in 0..2 {
            let frames: Vec<animation::Frame> =
                animation::frames(bytes.clone()).unwrap().collect::<anyhow::Result<_>>().unwrap();
            assert!(frames.len() >= 2, "{name}: {} frames", frames.len());
            assert_eq!(frames.iter().map(|f| f.delay_ms).sum::<u32>(), 400, "{name}");
            assert!(frames.iter().all(|f| (f.width, f.height) == (160, 80)), "{name}");
            let px = |f: &animation::Frame, x: usize, y: usize| {
                let i = (y * 160 + x) * 4;
                [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]]
            };
            let (first, last) = (&frames[0], &frames[frames.len() - 1]);
            assert!(near(px(first, 20, 40), RED) && near(px(first, 140, 40), BLUE), "{name}: {:?}", px(first, 20, 40));
            assert!(near(px(last, 20, 40), GREEN), "{name}: {:?}", px(last, 20, 40));
        }
    }

    for (name, codec) in [("h264.mp4", "h264"), ("vp9.webm", "vp9"), ("av1.mp4", "av1"), ("prores.mov", "prores")] {
        let path = media.join(name);
        let p = probe(&path).unwrap();
        assert_eq!((p.width, p.height, p.codec, p.orientation.get()), (160, 80, codec, 1), "{name}");
        assert!((2900..=3100).contains(&p.duration_ms), "{name}: {} ms", p.duration_ms);
        assert!((70..=80).contains(&p.frames), "{name}: {} frames", p.frames);
        let early = frame(&path, 500, Turns::new(0)).unwrap();
        assert_eq!((early.width, early.height), (160, 80));
        assert!(near(pixel(&early, 20, 40), RED) && near(pixel(&early, 140, 40), BLUE), "{name} at 0.5 s");
        let late = frame(&path, 2000, Turns::new(0)).unwrap();
        assert!(near(pixel(&late, 20, 40), GREEN), "{name} at 2 s: {:?}", pixel(&late, 20, 40));
        // Past the end: the last frame.
        assert!(near(pixel(&frame(&path, 60_000, Turns::new(0)).unwrap(), 80, 40), GREEN), "{name} past the end");
    }
    let h264 = probe(&media.join("h264.mp4")).unwrap();
    assert_eq!(h264.taken, Some(Taken::Utc(wall_clock_seconds(2022, 3, 4, 5, 6, 7).unwrap())));

    // Shown upright: 80 × 160, the stored left half (red) at the bottom.
    let rotated = media.join("rotated.mp4");
    let p = probe(&rotated).unwrap();
    assert_eq!((p.width, p.height, p.orientation.get()), (80, 160, 8));
    let f = frame(&rotated, 500, Turns::new(0)).unwrap();
    assert_eq!((f.width, f.height), (80, 160));
    assert!(near(pixel(&f, 40, 140), RED) && near(pixel(&f, 40, 20), BLUE));
    // A catalog turn on top: one more quarter turn clockwise brings it back to landscape, red on the left.
    let f = frame(&rotated, 500, Turns::new(1)).unwrap();
    assert_eq!((f.width, f.height), (160, 80));
    assert!(near(pixel(&f, 20, 40), RED), "{:?}", pixel(&f, 20, 40));

    // Mirrored: blue on the left. A catalog turn comes after the mirror: red at the top.
    let mirrored = media.join("mirrored.mp4");
    let p = probe(&mirrored).unwrap();
    assert_eq!((p.width, p.height, p.orientation.get()), (160, 80, 2));
    let f = frame(&mirrored, 500, Turns::new(0)).unwrap();
    assert!(near(pixel(&f, 20, 40), BLUE) && near(pixel(&f, 140, 40), RED), "{:?}", pixel(&f, 20, 40));
    let f = frame(&mirrored, 500, Turns::new(1)).unwrap();
    assert_eq!((f.width, f.height), (80, 160));
    assert!(near(pixel(&f, 40, 20), BLUE) && near(pixel(&f, 40, 140), RED), "{:?}", pixel(&f, 40, 20));

    // The stored orientation includes the catalog turn, as a still's does: the display matrix (8, and 2 mirrored)
    // composed with the turn. With a turn, the orientation is the one that shows the stored pixels turned.
    let mut analyzer = analyze::Analyzer::new().unwrap();
    for (path, turns, want) in [(&rotated, 0, 8), (&rotated, 1, 1), (&mirrored, 0, 2), (&mirrored, 1, 7)] {
        let a = analyze::video(path, Turns::new(turns), &mut analyzer).unwrap();
        assert_eq!(a.record.orientation.get(), want, "{} with {turns} turn(s)", path.display());
    }

    assert_eq!(poster_ms(3000), 300);
    assert_eq!(poster_ms(3_600_000), 10_000);

    // The library passes: scan, import, thumbnails.
    let mut catalog = Catalog::prepare(&dir.join("catalog.sqlite"), pg_library::versions(), PathRules::NATIVE).unwrap();
    let ctl = Control::new();
    let root = pg_scan::normalize_path(&media).unwrap();
    catalog.add_root(&root).unwrap();
    pg_scan::scan(&mut catalog, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = pg_import::import(&mut catalog, &opts, false, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed), (6, 0));
    let rows = state::media_rows(&catalog).unwrap();
    for (name, kind, w, h, duration, poster, no_pixel_hash) in &rows {
        assert_eq!(kind, "video", "{name}");
        assert!((2900..=3100).contains(duration), "{name}");
        assert_eq!((*poster, *no_pixel_hash), (poster_ms(*duration), Some(1)), "{name}");
        let want = if name == "rotated.mp4" { (80, 160) } else { (160, 80) };
        assert_eq!((*w, *h), want, "{name}");
    }
    // creation_time is UTC; the catalog keeps this computer's wall-clock time.
    let taken = state::taken_of(&catalog, "h264.mp4").unwrap();
    let local = state::local_epoch(&catalog, wall_clock_seconds(2022, 3, 4, 5, 6, 7).unwrap()).unwrap();
    assert_eq!(taken, Some(local));

    let (events, _quiet) = Quiet::bus();
    let cache = ThumbCache::open(&dir, &catalog.catalog_id().unwrap(), Some(1 << 30), events).unwrap();
    let made = pg_thumbs::run(&cache, &dir.join("catalog.sqlite"), 64, &opts, &ctl).unwrap();
    assert_eq!((made.made, made.failed), (6, 0));
}

/// A folder whose name ends in a dot is read through its verbatim path. Windows drops the dot from the plain path, which
/// would open the file of the sibling folder of the same name instead: the dotted folder's own file is the one read.
#[cfg(windows)]
#[test]
fn a_video_in_a_folder_ending_in_a_dot_is_read_from_that_folder() {
    let Some(exe) = ffmpeg() else {
        pg_testkit::missing_tool("ffmpeg");
        return;
    };
    let dir = clips(&exe).unwrap();
    let media = dir.join("media");
    let verbatim = pg_platform::native::verbatim;
    let dotted = dir.join("dotted").join("Acme Inc.");
    let plain = dir.join("dotted").join("Acme Inc");
    for folder in [&dotted, &plain] {
        std::fs::create_dir_all(verbatim(folder)).unwrap();
    }
    // The dotted folder holds the upright one (orientation 1), the plain folder the mirrored one (2).
    std::fs::copy(verbatim(&media.join("h264.mp4")), verbatim(&dotted.join("a.mp4"))).unwrap();
    std::fs::copy(verbatim(&media.join("mirrored.mp4")), verbatim(&plain.join("a.mp4"))).unwrap();
    let mut analyzer = analyze::Analyzer::new().unwrap();
    let a = analyze::video(&verbatim(&dotted.join("a.mp4")), Turns::new(0), &mut analyzer).unwrap();
    assert_eq!(a.record.orientation.get(), 1, "the dotted folder's own file");
}
