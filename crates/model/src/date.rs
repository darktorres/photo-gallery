//! Civil dates as the files and the catalog write them (model-shaped: pure arithmetic over calendar fields, no
//! time zone): an EXIF `DateTime` value, an ISO 8601 date or date and time, and the Unix seconds of a wall-clock
//! time. The times are wall-clock: a zone in the text is dropped, as the camera's clock showed it. A calendar day
//! ([`Day`]) is the same count of days, for a query's date bounds.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

/// A calendar day, without a time zone: the days since 1970-01-01 (negative before it).
#[cfg_attr(feature = "specta", derive(specta::Type), specta(type = String))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Day(i64);

impl Day {
    /// `YYYY-MM-DD` for a day that exists, with spaces around it ignored; `None` for anything else.
    pub fn parse(text: &str) -> Option<Day> {
        let t = text.trim();
        let b = t.as_bytes();
        let shaped = b.len() == 10 && b[4] == b'-' && b[7] == b'-';
        if !shaped || !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
            return None;
        }
        let num = |r: std::ops::Range<usize>| t[r].parse::<i64>().ok();
        let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        wall_clock_seconds(y, m, d, 0, 0, 0).map(|s| Day(s.div_euclid(86_400)))
    }

    /// Midnight at the start of the day, in Unix seconds of the wall clock.
    pub fn seconds(self) -> i64 {
        self.0 * 86_400
    }
}

impl std::fmt::Display for Day {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (y, m, d) = civil(self.0);
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl Serialize for Day {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Day {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Day, D::Error> {
        let text = String::deserialize(deserializer)?;
        Day::parse(&text).ok_or_else(|| <D::Error as serde::de::Error>::custom(format!("not a day: {text}")))
    }
}

/// The civil year, month and day of a day count from 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_parses_only_as_a_day_that_exists() {
        assert_eq!(Day::parse("2024-02-29").map(Day::seconds), wall_clock_seconds(2024, 2, 29, 0, 0, 0));
        assert_eq!(Day::parse(" 2000-12-31 "), Day::parse("2000-12-31"));
        assert_eq!(Day::parse("1969-12-31").map(Day::seconds), Some(-86_400));
        for bad in ["2023-02-29", "1900-02-29", "2000-13-01", "2000-00-10", "2000-1-1", "2000/01/01", "2000-01-1x"] {
            assert_eq!(Day::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_day_is_written_as_it_parses() {
        // Years 1 to 9999, the range the wall clock takes.
        for days in (-719_162..2_932_897).step_by(1_357) {
            let day = Day(days);
            assert_eq!(Day::parse(&day.to_string()), Some(day), "{day}");
        }
        assert_eq!(Day(0).to_string(), "1970-01-01");
    }
}
