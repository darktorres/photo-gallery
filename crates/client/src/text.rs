//! The words the front end shows: the facet labels and the filter line, the grid's badge and info line, the jobs' status
//! and notice lines, the write-back line, the Yandex panel's lines, the rating symbols and the timecodes. Each function
//! takes the values it words as plain data (model types, numbers, strings), so this crate needs no Qt and no job type;
//! the desktop maps the job's values in. Nothing here reads the catalog.

use pg_model::detection::Class;
use pg_model::query::{DateRange, MediaType, Query, Range, RatingFilter};

use crate::input::day_text;

/// The rating symbols, by rating (0 unrated, 5 best).
pub const RATING_SYMBOLS: [&str; 6] = ["☆", "❌", "❔", "🔨", "✔️", "⭐"];

/// The symbol of a rating; a rating past 5 is shown as 5.
pub fn rating_symbol(rating: u8) -> &'static str {
    RATING_SYMBOLS[usize::from(rating.min(5))]
}

/// A playback position or a length in whole seconds: m:ss, or h:mm:ss from an hour.
pub fn timecode(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// How the filter bar names a value of a list facet.
pub fn facet_label(facet: &str, value: &str) -> String {
    match (facet, value) {
        ("types", "image") => "Image",
        ("types", "animated") => "Animated",
        ("types", "video") => "Video",
        ("orientations", "landscape") => "Landscape",
        ("orientations", "portrait") => "Portrait",
        ("orientations", "square") => "Square",
        ("status", "imported") => "Imported",
        ("status", "pending") => "Not imported yet",
        ("status", "failed") => "Unreadable",
        ("borders", "none") => "None",
        ("borders", "black") => "Black",
        ("borders", "white") => "White",
        ("borders", "color") => "Colored",
        ("borders", "blur") => "Blurred bars",
        ("exts", "") => "(none)",
        (_, other) => other,
    }
    .to_string()
}

/// What `q` keeps, in one line: "all files" when it filters nothing.
pub fn describe(q: &Query) -> String {
    let open = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
    let range = |r: &Range, unit: &str| format!("{}–{}{unit}", open(r.min), open(r.max));
    let days = |r: &DateRange| format!("{} – {}", day_text(r.from), day_text(r.to));
    let mut parts: Vec<String> = Vec::new();
    if !q.types.is_empty() {
        parts.push(q.types.iter().map(|t| facet_label("types", t.key())).collect::<Vec<_>>().join(", "));
    }
    if !q.exts.is_empty() {
        parts.push(q.exts.join(", "));
    }
    if !q.ratings.is_empty() {
        let ratings: Vec<String> = q
            .ratings
            .iter()
            .map(|&r| match r {
                RatingFilter::Unrated => "unrated".to_string(),
                RatingFilter::Stars(stars) => rating_symbol(stars.get()).to_string(),
            })
            .collect();
        parts.push(ratings.join(" "));
    }
    if q.size_mb.min.is_some() || q.size_mb.max.is_some() {
        parts.push(range(&q.size_mb, " MB"));
    }
    if q.megapixels.min.is_some() || q.megapixels.max.is_some() {
        parts.push(range(&q.megapixels, " MP"));
    }
    if !q.shapes.is_empty() {
        parts.push(q.shapes.iter().map(|o| o.key()).collect::<Vec<_>>().join(", "));
    }
    if q.taken.from.is_some() || q.taken.to.is_some() {
        parts.push(format!("taken {}", days(&q.taken)));
    }
    if q.modified.from.is_some() || q.modified.to.is_some() {
        parts.push(format!("modified {}", days(&q.modified)));
    }
    if q.duration_s.min.is_some() || q.duration_s.max.is_some() {
        parts.push(range(&q.duration_s, " s"));
    }
    if let Some(d) = q.duplicates {
        parts.push(if d { "with duplicates" } else { "without duplicates" }.to_string());
    }
    if !q.status.is_empty() {
        parts.push(
            q.status.iter().map(|s| facet_label("status", s.key()).to_lowercase()).collect::<Vec<_>>().join(", "),
        );
    }
    if !q.borders.is_empty() {
        let values: Vec<String> = q.borders.iter().map(|b| facet_label("borders", b.key()).to_lowercase()).collect();
        parts.push(format!("borders: {}", values.join(", ")));
    }
    if let Some(m) = q.border_marked {
        parts.push(if m { "borders marked not a border" } else { "borders not marked" }.to_string());
    }
    if !q.name.is_empty() {
        parts.push(format!("name has “{}”", q.name));
    }
    if parts.is_empty() { "all files".to_string() } else { parts.join(" · ") }
}

/// The badge on a grid cell: a video's length, an animation's format; nothing for a still image.
pub fn badge(media: MediaType, duration_ms: Option<u32>, ext: &str) -> String {
    match media {
        MediaType::Video => match duration_ms {
            Some(ms) => format!("▶ {}", timecode((u64::from(ms) + 500) / 1000)),
            None => "▶".to_string(),
        },
        MediaType::Animated if ext == "png" => "APNG".to_string(),
        MediaType::Animated => ext.to_uppercase(),
        MediaType::Image => String::new(),
    }
}

/// The borders of a file that a crop would cut: whether the user marked them not a border, the cut in stored pixels
/// (left, top, right, bottom), and the classes of the accepted sides.
pub struct FoundBorders<'a> {
    pub marked: bool,
    pub cut: [u32; 4],
    pub classes: &'a [Class],
}

/// The info line's part about borders; empty when there are none to show.
pub fn borders_info(borders: Option<FoundBorders<'_>>) -> String {
    let Some(b) = borders else { return String::new() };
    let mut names: Vec<&str> = Vec::new();
    for c in b.classes {
        let name = match c {
            Class::Black => "black",
            Class::White => "white",
            Class::Color => "colored",
            Class::Blur => "blurred",
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let [l, t, r, bottom] = b.cut;
    let marked = if b.marked { " — marked not a border" } else { "" };
    format!("   borders {} (L {l} T {t} R {r} B {bottom} px){marked}", names.join(", "))
}

/// A file's info line: its path, its size on screen and in the file, and the notes on its rating and borders.
pub fn info(
    path: &str,
    shown: (u32, u32),
    bytes: u64,
    rating_kept_in_catalog: bool,
    borders: Option<FoundBorders<'_>>,
) -> String {
    let (w, h) = shown;
    let dims = if w > 0 { format!("{w} × {h}") } else { "not imported".to_string() };
    let bytes = bytes as f64;
    let size = if bytes >= 1e6 { format!("{:.1} MB", bytes / 1e6) } else { format!("{:.0} KB", bytes / 1e3) };
    let kept = if rating_kept_in_catalog { "   rating kept in the catalog" } else { "" };
    format!("{path}   {dims}   {size}{kept}{}", borders_info(borders))
}

/// A pass the job runs, as the status line names it.
pub enum Pass<'a> {
    Idle,
    Scan { root: &'a str },
    Import,
    Borders,
    Crop,
    Thumbs { height: u32 },
}

/// A pass's counts: its files done and in all, the failures, the rate (files a second) and the time left (seconds).
pub struct Counts {
    pub done: u64,
    pub total: u64,
    pub failed: u64,
    pub rate: f64,
    pub eta_secs: f64,
}

/// The status line's text for a pass's progress; empty when idle.
pub fn pass_status(pass: Pass<'_>, c: &Counts) -> String {
    match pass {
        Pass::Idle => String::new(),
        Pass::Scan { root } => format!("Scanning {root} · {} images found", group_digits(c.done)),
        Pass::Import => working("Importing", c),
        Pass::Borders => working("Finding borders", c),
        Pass::Crop => working("Saving cropped copies", c),
        Pass::Thumbs { height } => working(&format!("Thumbnails at {height} px"), c),
    }
}

/// A pass that counts its files: how far it is, its rate, and what is left.
fn working(what: &str, c: &Counts) -> String {
    let failed = if c.failed > 0 { format!(" · {} failed", group_digits(c.failed)) } else { String::new() };
    format!(
        "{what} {} of {} · {:.0}/s · {}{failed}",
        group_digits(c.done),
        group_digits(c.total),
        c.rate,
        duration(c.eta_secs)
    )
}

/// A count with its thousands grouped: 1,234,567.
pub fn group_digits(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A time left, in seconds, as the status line says it: "42 s left", "3 min left", "1 h 5 min left".
pub fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..60 => format!("{s} s left"),
        60..3600 => format!("{} min left", s.div_ceil(60)),
        _ => format!("{} h {} min left", s / 3600, s % 3600 / 60),
    }
}

/// The write-back line: the changes not yet in the files, or the latest failure; empty when there are none.
pub fn write_status(queued: usize, error: Option<&str>) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    match (queued, error) {
        (0, None) => String::new(),
        (n, None) => format!("Writing {n} change{} into files", plural(n)),
        (n, Some(e)) => format!("{n} change{} not yet in the files: {e}", plural(n)),
    }
}

/// A root whose folders a scan couldn't read.
pub fn unreadable_folders(root: &str, count: u64) -> String {
    format!("{root}: {count} folders couldn't be read")
}

/// A scan that failed.
pub fn scan_failed(root: &str, error: &str) -> String {
    format!("{root}: {error}")
}

/// A round of the job that panicked (caught).
pub fn round_panicked(message: &str) -> String {
    format!("internal error in the background job: {message}")
}

/// The thumbnail cache is full: `made` thumbnails at `height` px weren't made.
pub fn cache_full(height: u32, made: usize) -> String {
    format!("The thumbnail cache is full: {made} thumbnails at {height} px weren't made")
}

/// A crop job that ran to the end of its request: what it saved in `folder`, what it skipped and failed, and the first
/// error when some failed.
pub fn cropped(saved: usize, folder: &str, skipped: usize, failed: usize, first_error: Option<&str>) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut note = format!("Saved {saved} cropped cop{} in {folder}", if saved == 1 { "y" } else { "ies" });
    if skipped > 0 {
        note += &format!("; {skipped} file{} had nothing to crop any more", plural(skipped));
    }
    if let Some(e) = first_error {
        note += &format!("; {failed} failed, the first: {e}");
    }
    note
}

/// Yandex asked for a captcha. Solving it in a browser doesn't help: the app doesn't share the browser's cookies.
pub const YANDEX_CAPTCHA: &str = "Yandex asked for a captcha, which the app can't answer. Try again later.";

/// The Yandex search takes still images only.
pub const YANDEX_NOT_STILL: &str = "Yandex search takes still images (JPEG, PNG, WebP, GIF or BMP).";

pub const YANDEX_UPLOADING: &str = "Uploading the photo…";
pub const YANDEX_OTHER_SIZES: &str = "Reading the other sizes…";
pub const YANDEX_SIMILAR: &str = "Reading similar images…";

/// The photo to search couldn't be read.
pub fn yandex_unreadable_photo(error: &str) -> String {
    format!("Can't read the photo: {error}")
}

/// The similar images stopped after `shown` results, for `reason`.
pub fn yandex_similar_stopped(shown: usize, reason: &str) -> String {
    format!("Similar images stopped after {shown} results: {reason}")
}

/// The similar images failed, for `reason` (the other sizes are still shown).
pub fn yandex_similar_failed(reason: &str) -> String {
    format!("Similar images failed: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes_drop_the_hour_until_there_is_one() {
        assert_eq!(timecode(0), "0:00");
        assert_eq!(timecode(65), "1:05");
        assert_eq!(timecode(3599), "59:59");
        assert_eq!(timecode(3600), "1:00:00");
        assert_eq!(timecode(3725), "1:02:05");
    }

    #[test]
    fn ratings_past_five_show_the_best_symbol() {
        assert_eq!(rating_symbol(0), "☆");
        assert_eq!(rating_symbol(5), "⭐");
        assert_eq!(rating_symbol(9), "⭐");
    }

    #[test]
    fn a_video_badge_rounds_its_length_to_the_second() {
        assert_eq!(badge(MediaType::Video, Some(1499), "mp4"), "▶ 0:01");
        assert_eq!(badge(MediaType::Video, Some(1500), "mp4"), "▶ 0:02");
        assert_eq!(badge(MediaType::Video, None, "mp4"), "▶");
        assert_eq!(badge(MediaType::Animated, None, "png"), "APNG");
        assert_eq!(badge(MediaType::Animated, None, "gif"), "GIF");
        assert_eq!(badge(MediaType::Image, None, "jpg"), "");
    }

    #[test]
    fn counts_are_grouped_and_times_left_are_named() {
        assert_eq!(group_digits(1_234_567), "1,234,567");
        assert_eq!(group_digits(999), "999");
        assert_eq!(duration(42.4), "42 s left");
        assert_eq!(duration(61.0), "2 min left");
        assert_eq!(duration(3900.0), "1 h 5 min left");
    }

    #[test]
    fn the_write_line_counts_its_changes() {
        assert_eq!(write_status(0, None), "");
        assert_eq!(write_status(1, None), "Writing 1 change into files");
        assert_eq!(write_status(3, Some("offline")), "3 changes not yet in the files: offline");
    }

    #[test]
    fn a_pass_line_says_how_far_it_is() {
        let c = Counts { done: 1500, total: 4000, failed: 2, rate: 100.0, eta_secs: 25.0 };
        assert_eq!(pass_status(Pass::Import, &c), "Importing 1,500 of 4,000 · 100/s · 25 s left · 2 failed");
        assert_eq!(pass_status(Pass::Idle, &c), "");
    }

    #[test]
    fn the_crop_note_pluralises() {
        assert_eq!(cropped(1, "D:/out", 0, 0, None), "Saved 1 cropped copy in D:/out");
        assert_eq!(
            cropped(2, "D:/out", 2, 1, Some("disk full")),
            "Saved 2 cropped copies in D:/out; 2 files had nothing to crop any more; 1 failed, the first: disk full"
        );
    }

    #[test]
    fn an_info_line_names_what_the_catalog_keeps() {
        let line = info("D:/a.jpg", (0, 0), 2_500_000, true, None);
        assert_eq!(line, "D:/a.jpg   not imported   2.5 MB   rating kept in the catalog");
    }
}
