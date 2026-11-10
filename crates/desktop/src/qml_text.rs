//! A check of the QML's text: a `Label`, `Text` or `TextArea` shows its text as rich text when it looks like markup (Qt's
//! `AutoText`), so a file, folder or view name, a path or an error holding `<b>` or `<img>` would be drawn as HTML (and
//! defeat the elision). Each one whose text isn't fixed words (a `words.text` or `words.phrase` call, or a literal) sets
//! its `textFormat`: `Text.PlainText`, unless it means otherwise.

use std::fs;
use std::path::Path;

/// The end of the block whose `{` is at `open` (the index of its `}`), skipping strings.
fn block_end(src: &[u8], open: usize) -> usize {
    let (mut depth, mut i, mut quote) = (0usize, open, None);
    while let Some(&c) = src.get(i) {
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    src.len()
}

/// A block's own text: its nested blocks left out.
fn own_text(body: &str) -> String {
    let (mut out, mut depth, mut quote) = (String::new(), 0usize, None);
    for c in body.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '{' {
            depth += 1;
        }
        if depth == 0 {
            out.push(c);
        }
        if quote.is_none() && c == '}' {
            depth = depth.saturating_sub(1);
        }
    }
    out
}

/// Whether a `text:` value is fixed words: one `words.text(..)` or `words.phrase(..)` call, or a string literal.
fn is_fixed(value: &str) -> bool {
    let value = value.trim();
    let call = ["words.text(", "words.phrase("].iter().any(|head| value.starts_with(head))
        && value.ends_with(')')
        && value.matches('(').count() == 1;
    let literal = value.len() >= 2 && value.starts_with('"') && value.ends_with('"') && value.matches('"').count() == 2;
    call || literal
}

/// The `text:` value of a block's own text (up to the line's end or a `;`).
fn text_value(own: &str) -> Option<String> {
    let mut rest = own;
    while let Some(at) = rest.find("text:") {
        let before = rest.get(..at).and_then(|b| b.chars().last());
        let after = rest.get(at + "text:".len()..).unwrap_or_default();
        if before.is_none_or(|c| c.is_whitespace() || c == ';' || c == '{') {
            let line = after.lines().next().unwrap_or_default();
            return Some(line.split(';').next().unwrap_or_default().trim().to_string());
        }
        rest = after;
    }
    None
}

#[test]
#[expect(
    clippy::string_slice,
    reason = "the indices are from match_indices and find over ASCII patterns: char boundaries"
)]
fn every_label_that_shows_a_value_is_plain_text() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("qml");
    let mut unmarked = Vec::new();
    for file in fs::read_dir(&dir).expect("the QML folder") {
        let path = file.expect("a QML file").path();
        if path.extension().is_none_or(|e| e != "qml") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("a readable QML file");
        for kind in ["Label", "Text", "TextArea"] {
            for (at, _) in src.match_indices(kind) {
                let word_start = at == 0 || !src.as_bytes()[at - 1].is_ascii_alphanumeric();
                let rest = &src[at + kind.len()..];
                let Some(open) = rest.find(|c: char| !c.is_whitespace()).filter(|i| rest[*i..].starts_with('{')) else {
                    continue;
                };
                if !word_start {
                    continue;
                }
                let open = at + kind.len() + open;
                let end = block_end(src.as_bytes(), open);
                let own = own_text(&src[open + 1..end]);
                if let Some(value) = text_value(&own)
                    && !is_fixed(&value)
                    && !own.contains("textFormat:")
                {
                    let line = src[..at].matches('\n').count() + 1;
                    unmarked.push(format!("{}:{line}: {value}", path.file_name().unwrap().to_string_lossy()));
                }
            }
        }
    }
    assert!(unmarked.is_empty(), "these show a value without a textFormat (Text.PlainText): {unmarked:#?}");
}

#[test]
fn fixed_words_need_no_text_format_and_values_do() {
    assert!(is_fixed(r#"words.text("ui-close")"#));
    assert!(is_fixed(r#""1:1""#));
    assert!(!is_fixed("bar.message"));
    assert!(!is_fixed(r#"bar.paused ? words.text("ui-resume") : words.text("ui-pause")"#));
    assert_eq!(text_value("id: x\n    text: row.message\n").as_deref(), Some("row.message"));
    assert_eq!(text_value(" text: a; color: b").as_deref(), Some("a"));
    assert_eq!(text_value("ToolTip.text: a"), None);
}
