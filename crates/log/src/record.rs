//! A record of the log and its line in the file: one JSON object per line. The reader also takes the plain lines the
//! app wrote before (`2026-10-30 17:04:16 text`), and the lines a plain record ran on to (a panic's backtrace).

use pg_model::notice::Level;
use serde::{Deserialize, Serialize};

/// The most bytes of a record's message kept.
pub const MESSAGE_CAP: usize = 4096;
/// The most bytes of a record's detail kept. 16 KiB, not the message's 4: a panic's backtrace is a detail, and one
/// passes 4 KiB (a few dozen frames, each with its file and line), and the frames that place the panic are its end.
pub const DETAIL_CAP: usize = 16 * 1024;

/// One thing the app logged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// When, in milliseconds since 1970 (UTC); none for a plain line of an older build.
    #[serde(rename = "t", default, skip_serializing_if = "Option::is_none")]
    pub t_ms: Option<u64>,
    /// When, as the local time it was written at (`2026-10-30 17:04:16`): what the log view shows.
    pub local: String,
    pub level: Level,
    /// What the record is about: `session`, `library`, `video`, `panic`, ... (free text, a word).
    pub target: String,
    #[serde(rename = "msg")]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The process that wrote it, and when that process began (its session): with `seq`, a record's identity.
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub session: u64,
    /// The record's number in its session, from 0.
    #[serde(default)]
    pub seq: u64,
    /// Written to the file only, not kept in memory nor sent: a panic raised while its thread held the records in
    /// memory (`RingGuard`), which can't be taken again on that thread. A subscriber's merge keeps it from the file
    /// ([`crate::Feed::merge`]).
    #[serde(default, skip_serializing_if = "is_false")]
    pub file_only: bool,
}

/// For `file_only`'s `skip_serializing_if`, which passes a reference: a record that isn't file-only writes no field.
fn is_false(value: &bool) -> bool {
    !*value
}

impl Record {
    /// The record's line in the file, with its newline.
    pub(crate) fn line(&self) -> String {
        // A record of strings and numbers always serialises; the fallback keeps the line a line.
        let mut line = serde_json::to_string(self).unwrap_or_else(|e| format!(r#"{{"msg":"unwritable record: {e}"}}"#));
        line.push('\n');
        line
    }

    /// The record as a person reads it on stderr: `local level target: message`, then its detail.
    pub(crate) fn plain(&self) -> String {
        let mut text = format!("{} {:?} {}: {}\n", self.local, self.level, self.target, self.message);
        if let Some(detail) = &self.detail {
            text.push_str(detail);
            text.push('\n');
        }
        text
    }

    /// Its size in memory, for the ring's bound: its texts.
    pub(crate) fn bytes(&self) -> usize {
        self.local.len() + self.target.len() + self.message.len() + self.detail.as_ref().map_or(0, String::len)
    }
}

/// The records of a log file's text, oldest first. A JSON line is a record; a plain line that starts with a date and a
/// time is a record of an older build (info, its time as written); any other line continues the record before it (a
/// plain panic's backtrace), or is a record of its own when it starts the text. A JSON line cut short (the app ended in
/// the middle of it) reads as a plain line.
pub fn parse(text: &str) -> Vec<Record> {
    let mut records: Vec<Record> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('{')
            && let Ok(record) = serde_json::from_str::<Record>(line)
        {
            records.push(record);
            continue;
        }
        match (legacy_time(line), records.last_mut()) {
            (Some((local, message)), _) => records.push(plain_record(local, message)),
            (None, Some(last)) if last.t_ms.is_none() => {
                let detail = last.detail.get_or_insert_with(String::new);
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(line);
            }
            (None, _) => records.push(plain_record(String::new(), line.to_string())),
        }
    }
    records
}

fn plain_record(local: String, message: String) -> Record {
    Record {
        t_ms: None,
        local,
        level: Level::Info,
        target: String::new(),
        message,
        detail: None,
        pid: 0,
        session: 0,
        seq: 0,
        file_only: false,
    }
}

/// `YYYY-MM-DD HH:MM:SS rest`: the time and the rest, for a line an older build wrote.
fn legacy_time(line: &str) -> Option<(String, String)> {
    let (time, rest) = (line.get(..19)?, line.get(19..)?);
    let digits = |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    let shaped = digits(time.get(0..4))
        && time.get(4..5) == Some("-")
        && digits(time.get(5..7))
        && time.get(7..8) == Some("-")
        && digits(time.get(8..10))
        && time.get(10..11) == Some(" ")
        && digits(time.get(11..13))
        && time.get(13..14) == Some(":")
        && digits(time.get(14..16))
        && time.get(16..17) == Some(":")
        && digits(time.get(17..19));
    shaped.then(|| (time.to_string(), rest.strip_prefix(' ').unwrap_or(rest).to_string()))
}

/// The mark a cut text ends in.
const ELLIPSIS: char = '…';

/// `text` cut to at most `cap` bytes, at a character's edge, ending in `…` when cut: the mark is within the cap, so the
/// result is never more than `cap` bytes (under the mark's 3 bytes, a cut text is the characters that fit, with no
/// mark).
pub(crate) fn capped(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let Some(room) = cap.checked_sub(ELLIPSIS.len_utf8()) else {
        return prefix(text, cap).to_string();
    };
    let mut out = prefix(text, room).to_string();
    out.push(ELLIPSIS);
    out
}

/// The longest start of `text` of at most `bytes` bytes that ends at a character's edge.
fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    // 0 is an edge, so this stops.
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: u64) -> Record {
        Record {
            t_ms: Some(1_700_000_000_000 + seq),
            local: "2026-11-10 12:00:00".into(),
            level: Level::Warning,
            target: "library".into(),
            message: format!("record {seq}"),
            detail: Some("line one\nline two".into()),
            pid: 7,
            session: 1,
            seq,
            file_only: false,
        }
    }

    #[test]
    fn a_record_reads_back_from_its_line() {
        let r = record(3);
        assert_eq!(parse(&r.line()), vec![r]);
    }

    #[test]
    fn a_record_is_one_line_whatever_its_text_holds() {
        let line = record(1).line();
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn an_older_builds_plain_lines_read_as_info_with_their_time() {
        let text = "2026-10-30 17:04:16 data folder: D:\\Gallery\n2026-10-30 17:04:17 library: x\n";
        let records = parse(text);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].local, "2026-10-30 17:04:16");
        assert_eq!(records[0].message, "data folder: D:\\Gallery");
        assert_eq!(records[0].level, Level::Info);
        assert_eq!(records[0].t_ms, None);
    }

    /// The panic hook of an older build wrote the backtrace on the lines after its report.
    #[test]
    fn a_plain_panics_backtrace_is_its_detail() {
        let text = "2026-10-30 17:04:16 gallery 0.1.0, thread 'main': panicked at x\n   0: std::panic\n   1: main\n\n";
        let records = parse(text);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].detail.as_deref(), Some("   0: std::panic\n   1: main"));
    }

    #[test]
    fn plain_and_json_lines_mix_in_file_order() {
        let text = format!("2026-10-30 17:04:16 old\n{}2026-10-30 17:04:18 older build again\n", record(0).line());
        let messages: Vec<String> = parse(&text).into_iter().map(|r| r.message).collect();
        assert_eq!(messages, ["old", "record 0", "older build again"]);
    }

    /// The app ended in the middle of a line: the part written is kept, as a plain record.
    #[test]
    fn a_truncated_last_line_is_kept_as_text() {
        let whole = record(0).line();
        let cut = whole.get(..whole.len() / 2).unwrap();
        let text = format!("{whole}{cut}");
        let records = parse(&text);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].message, cut);
        assert_eq!(records[1].t_ms, None);
    }

    /// A file-only record says so in its line; any other writes no field, and an older line reads as not file-only.
    #[test]
    fn a_file_only_record_says_so_and_no_other_does() {
        assert!(!record(1).line().contains("file_only"));
        let panic = Record { file_only: true, ..record(2) };
        assert!(panic.line().contains(r#""file_only":true"#));
        assert_eq!(parse(&panic.line()), vec![panic]);
    }

    #[test]
    fn a_capped_text_ends_at_a_character_with_its_mark_inside_the_cap() {
        assert_eq!(capped("abc", 3), "abc");
        assert_eq!(capped("abcd", 3), "…");
        assert_eq!(capped("abcdef", 5), "ab…");
        // é is 2 bytes, € 3, the mark 3.
        assert_eq!(capped("aéb", 4), "aéb");
        assert_eq!(capped("aébc", 4), "a…");
        assert_eq!(capped("aébcd", 5), "a…");
        assert_eq!(capped("aébcde", 6), "aé…");
        assert_eq!(capped("ééé", 5), "é…");
        assert_eq!(capped("a€€", 6), "a…");
        assert_eq!(capped("a€€", 7), "a€€");
        // Under the mark's bytes, the characters that fit.
        assert_eq!(capped("aéb", 2), "a");
        assert_eq!(capped("éa", 1), "");
    }

    /// At every cap, a capped text is at most the cap, and is the text's start (with the mark when cut).
    #[test]
    fn a_capped_text_is_never_past_its_cap() {
        for text in ["", "abc", "ééé€a", "a€é\nb", "€€€€", "😀a😀"] {
            for cap in 0..=text.len() + 1 {
                let out = capped(text, cap);
                assert!(out.len() <= cap, "{text:?} at {cap}: {out:?}");
                if text.len() <= cap {
                    assert_eq!(out, text);
                } else {
                    let start = out.strip_suffix(ELLIPSIS).unwrap_or(&out);
                    assert!(text.starts_with(start), "{text:?} at {cap}: {out:?}");
                    assert_eq!(out.ends_with(ELLIPSIS), cap >= ELLIPSIS.len_utf8(), "{text:?} at {cap}: {out:?}");
                }
            }
        }
    }
}
