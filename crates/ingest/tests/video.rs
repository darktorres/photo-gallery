//! Videos through FFmpeg: probing (size, rotation, duration, codec, date), the frame at a time, and a scan,
//! import and thumbnail pass over a folder of them. Also the viewer's animations (`pg_codec::animation`): WebP, GIF
//! and APNG made by FFmpeg, frame by frame. The clips are made with the `ffmpeg` command line (`PG_FFMPEG`,
//! else on PATH; the test is skipped without it), in a scratch folder (`PG_TEST_DIR`, default
//! `D:\PhotoGallery\scratch_video`, never C:).
//!
//! Every clip is 160 × 80 at 25 fps for 3 s: red on the left half and blue on the right for the first second, then
//! all green.

use std::path::{Path, PathBuf};
use std::process::Command;

use pg_analysis::analyze;
use pg_catalog::Catalog;
use pg_codec::animation;
use pg_codec::pixels::Rgb;
use pg_ingest::control::Control;
use pg_ingest::pipeline::Options;
use pg_ingest::{import, scan, thumbs};
use pg_model::analysis::Taken;
use pg_model::date::wall_clock_seconds;
use pg_thumbcache::ThumbCache;
use pg_video::{frame, poster_ms, probe};

const SCENE: &str = "color=c=red:s=80x80:r=25:d=3[a];color=c=blue:s=80x80:r=25:d=3[b];[a][b]hstack[ab];\
                     color=c=green:s=160x80:r=25:d=3[g];[ab][g]overlay=enable='gte(t,1)',format=yuv420p";

fn ffmpeg() -> Option<PathBuf> {
    let exe = std::env::var_os("PG_FFMPEG").map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
    Command::new(&exe).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| exe)
}

fn run(exe: &Path, args: &[&str]) {
    let out = Command::new(exe).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// The clips, made once per run.
fn clips(exe: &Path) -> PathBuf {
    let dir = pg_testkit::run_folder(r"D:\PhotoGallery\scratch_video", "video");
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let out = |name: &str| media.join(name).to_string_lossy().into_owned();
    let encode = |name: &str, codec: &[&str]| {
        let mut args = vec!["-f", "lavfi", "-i", SCENE];
        args.extend_from_slice(codec);
        let path = out(name);
        args.push(&path);
        run(exe, &args);
    };
    encode("h264.mp4", &["-c:v", "libx264", "-metadata", "creation_time=2022-03-04T05:06:07Z"]);
    encode("vp9.webm", &["-c:v", "libvpx-vp9", "-b:v", "200k"]);
    encode("av1.mp4", &["-c:v", "libaom-av1", "-cpu-used", "8", "-b:v", "200k"]);
    encode("prores.mov", &["-c:v", "prores_ks"]);
    // A phone held upright: stored landscape, displayed turned 90° counterclockwise.
    run(exe, &["-display_rotation", "90", "-i", &out("h264.mp4"), "-c", "copy", &out("rotated.mp4")]);
    // A selfie camera's mirror: displayed flipped left to right.
    run(exe, &["-display_hflip", "-i", &out("h264.mp4"), "-c", "copy", &out("mirrored.mp4")]);
    // Animations: 0.8 to 1.2 s (red and blue, then green), outside the media folder.
    let anim = dir.join("anim");
    std::fs::create_dir_all(&anim).unwrap();
    for (name, codec) in [
        ("anim.webp", &["-c:v", "libwebp_anim", "-lossless", "1", "-loop", "0"][..]),
        ("anim.gif", &["-f", "gif"]),
        ("anim.png", &["-f", "apng", "-plays", "0"]),
    ] {
        let path = anim.join(name).to_string_lossy().into_owned();
        let mut args = vec!["-f", "lavfi", "-i", SCENE, "-ss", "0.8", "-t", "0.4"];
        args.extend_from_slice(codec);
        args.push(&path);
        run(exe, &args);
    }
    dir
}

fn pixel(img: &Rgb, x: usize, y: usize) -> [u8; 3] {
    let i = (y * img.width + x) * 3;
    [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]]
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
        eprintln!("skipped: no ffmpeg command line");
        return;
    };
    let dir = clips(&exe);
    let media = dir.join("media");

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
        assert_eq!((p.width, p.height, p.codec, p.orientation), (160, 80, codec, 1), "{name}");
        assert!((2900..=3100).contains(&p.duration_ms), "{name}: {} ms", p.duration_ms);
        assert!((70..=80).contains(&p.frames), "{name}: {} frames", p.frames);
        let early = frame(&path, 500, 0).unwrap();
        assert_eq!((early.width, early.height), (160, 80));
        assert!(near(pixel(&early, 20, 40), RED) && near(pixel(&early, 140, 40), BLUE), "{name} at 0.5 s");
        let late = frame(&path, 2000, 0).unwrap();
        assert!(near(pixel(&late, 20, 40), GREEN), "{name} at 2 s: {:?}", pixel(&late, 20, 40));
        // Past the end: the last frame.
        assert!(near(pixel(&frame(&path, 60_000, 0).unwrap(), 80, 40), GREEN), "{name} past the end");
    }
    let h264 = probe(&media.join("h264.mp4")).unwrap();
    assert_eq!(h264.taken, Some(Taken::Utc(wall_clock_seconds(2022, 3, 4, 5, 6, 7).unwrap())));

    // Shown upright: 80 × 160, the stored left half (red) at the bottom.
    let rotated = media.join("rotated.mp4");
    let p = probe(&rotated).unwrap();
    assert_eq!((p.width, p.height, p.orientation), (80, 160, 8));
    let f = frame(&rotated, 500, 0).unwrap();
    assert_eq!((f.width, f.height), (80, 160));
    assert!(near(pixel(&f, 40, 140), RED) && near(pixel(&f, 40, 20), BLUE));
    // A catalog turn on top: one more quarter turn clockwise brings it back to landscape, red on the left.
    let f = frame(&rotated, 500, 1).unwrap();
    assert_eq!((f.width, f.height), (160, 80));
    assert!(near(pixel(&f, 20, 40), RED), "{:?}", pixel(&f, 20, 40));

    // Mirrored: blue on the left. A catalog turn comes after the mirror: red at the top.
    let mirrored = media.join("mirrored.mp4");
    let p = probe(&mirrored).unwrap();
    assert_eq!((p.width, p.height, p.orientation), (160, 80, 2));
    let f = frame(&mirrored, 500, 0).unwrap();
    assert!(near(pixel(&f, 20, 40), BLUE) && near(pixel(&f, 140, 40), RED), "{:?}", pixel(&f, 20, 40));
    let f = frame(&mirrored, 500, 1).unwrap();
    assert_eq!((f.width, f.height), (80, 160));
    assert!(near(pixel(&f, 40, 20), BLUE) && near(pixel(&f, 40, 140), RED), "{:?}", pixel(&f, 40, 20));

    // The stored orientation includes the catalog turn, as a still's does: the display matrix (8, and 2 mirrored)
    // composed with the turn. With a turn, the orientation is the one that shows the stored pixels turned.
    let mut analyzer = analyze::Analyzer::new().unwrap();
    for (path, turns, want) in [(&rotated, 0, 8), (&rotated, 1, 1), (&mirrored, 0, 2), (&mirrored, 1, 7)] {
        let a = analyze::video(path, turns, &mut analyzer).unwrap();
        assert_eq!(a.orientation, want, "{} with {turns} turn(s)", path.display());
    }

    assert_eq!(poster_ms(3000), 300);
    assert_eq!(poster_ms(3_600_000), 10_000);

    // The library passes: scan, import, thumbnails.
    let mut catalog = Catalog::open(&dir.join("catalog.sqlite")).unwrap();
    let ctl = Control::new(false);
    let root = scan::normalize_path(&media.to_string_lossy()).unwrap();
    catalog.add_root(&root).unwrap();
    scan::scan(&mut catalog, &root, &ctl).unwrap();
    let opts = Options::new(pg_platform::priority::Priority::Normal);
    let stats = import::import(&mut catalog, &opts, false, &ctl).unwrap();
    assert_eq!((stats.files, stats.failed), (6, 0));
    // Name, kind, width, height, duration, poster time, whether the pixel hash is missing.
    type Row = (String, String, i64, i64, i64, i64, Option<i64>);
    let rows: Vec<Row> = catalog
        .conn
        .prepare(
            "SELECT f.name, f.kind, a.width, a.height, a.duration_ms, a.poster_ms, a.pixel_hash IS NULL
             FROM files f JOIN analysis a ON a.file_id = f.id ORDER BY f.name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (name, kind, w, h, duration, poster, no_pixel_hash) in &rows {
        assert_eq!(kind, "video", "{name}");
        assert!((2900..=3100).contains(duration), "{name}");
        assert_eq!((*poster, *no_pixel_hash), (poster_ms(*duration), Some(1)), "{name}");
        let want = if name == "rotated.mp4" { (80, 160) } else { (160, 80) };
        assert_eq!((*w, *h), want, "{name}");
    }
    // creation_time is UTC; the catalog keeps this computer's wall-clock time.
    let taken: Option<i64> = catalog
        .conn
        .query_row("SELECT taken FROM analysis a JOIN files f ON f.id = a.file_id WHERE f.name = 'h264.mp4'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let local: i64 = catalog
        .conn
        .query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [wall_clock_seconds(2022, 3, 4, 5, 6, 7)], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taken, Some(local));

    let cache =
        ThumbCache::open(&dir, &catalog.catalog_id().unwrap(), Some(1 << 30), Box::new(|e| panic!("{e:#}"))).unwrap();
    let made = thumbs::run(&cache, &dir.join("catalog.sqlite"), 64, &opts, &ctl).unwrap();
    assert_eq!((made.made, made.failed), (6, 0));
}
