//! Civil dates as the files and the catalog write them (model-shaped: pure arithmetic over calendar fields, no
//! time zone): an EXIF `DateTime` value, an ISO 8601 date or date and time, and the Unix seconds of a wall-clock
//! time. The times are wall-clock: a zone in the text is dropped, as the camera's clock showed it.

/// An EXIF date, `YYYY:MM:DD HH:MM:SS` (anything after it ignored), as wall-clock seconds; `None` when blank,
/// malformed or impossible. The format rules are kamadak-exif's (`DateTime::from_ascii`), which read them before.
pub fn exif_date(data: &[u8]) -> Option<i64> {
    if data == b"    :  :     :  :  " || data == b"                   " || data.len() < 19 {
        return None;
    }
    if [(4, b':'), (7, b':'), (10, b' '), (13, b':'), (16, b':')].iter().any(|&(i, c)| data[i] != c) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| {
        let digits = &data[r];
        digits.iter().all(u8::is_ascii_digit).then(|| digits.iter().fold(0, |n, &d| n * 10 + i64::from(d - b'0')))
    };
    wall_clock_seconds(num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?)
}

/// `YYYY-MM-DD`, optionally followed by `THH:MM[:SS[.fff]]` and a zone (ignored: the wall-clock time is kept).
pub fn parse_iso_date(s: &str) -> Option<i64> {
    let num = |s: &str| -> Option<i64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let (date, time) = s.trim().split_once('T').unwrap_or((s.trim(), ""));
    let mut d = date.splitn(3, '-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (mut h, mut mi, mut sec) = (0, 0, 0);
    if !time.is_empty() {
        let clock = time.split(['Z', '+', '-']).next()?;
        let mut t = clock.splitn(3, ':');
        h = num(t.next()?)?;
        mi = num(t.next()?)?;
        if let Some(s) = t.next() {
            sec = num(s.split('.').next()?)?;
        }
    }
    wall_clock_seconds(y, m, day, h, mi, sec)
}

/// Seconds since 1970-01-01 00:00 of a calendar date and time (proleptic Gregorian, no time zone); `None` for an
/// impossible one (EXIF's blank `0000:00:00 00:00:00` included).
pub fn wall_clock_seconds(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > month_days[m as usize - 1] {
        return None;
    }
    if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..=60).contains(&s) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}
