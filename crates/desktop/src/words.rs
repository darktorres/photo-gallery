//! The words QML shows that it can't make itself, from `pg_client::text`: the rating symbols, the timecodes, and the
//! front end's own literals (the labels and tooltips, by their `ui.ftl` id). One object per component that needs them,
//! made inside it (as the other facades are).

pub(crate) mod state;

use crate::words_bridge::qobject;

use cxx_qt_lib::QString;
use pg_client::text;

impl qobject::Words {
    pub(crate) fn rating_symbol(&self, rating: i32) -> QString {
        let rating = u8::try_from(rating.clamp(0, 5)).unwrap_or(0);
        QString::from(&text::rating_symbol(rating).to_string())
    }

    pub(crate) fn timecode(&self, seconds: i32) -> QString {
        QString::from(&text::timecode(u64::try_from(seconds).unwrap_or(0)))
    }

    pub(crate) fn text(&self, id: &QString) -> QString {
        QString::from(&text::ui(&String::from(id), 0.0, "", ""))
    }

    pub(crate) fn phrase(&self, id: &QString, count: f64, value: &QString, second: &QString) -> QString {
        QString::from(&text::ui(&String::from(id), count, &String::from(value), &String::from(second)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    /// The ids the QML asks the facade for: the first argument of each `words.text(` and `words.phrase(` call.
    #[expect(clippy::string_slice, reason = "a match position plus an ASCII call's length is a char boundary")]
    fn ids_in_qml() -> BTreeSet<String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("qml");
        let mut ids = BTreeSet::new();
        for file in fs::read_dir(&dir).expect("the QML folder") {
            let path = file.expect("a QML file").path();
            if path.extension().is_none_or(|e| e != "qml") {
                continue;
            }
            let source = fs::read_to_string(&path).expect("a readable QML file");
            for call in ["words.text(\"", "words.phrase(\""] {
                for (at, _) in source.match_indices(call) {
                    let rest = &source[at + call.len()..];
                    let id = rest.split('"').next().unwrap_or_default();
                    ids.insert(id.to_string());
                }
            }
        }
        ids
    }

    /// The entries of `ui.ftl`: the lines that start an entry (not a comment, a continuation or a variant).
    fn entries() -> BTreeSet<String> {
        include_str!("../../client/locales/en/ui.ftl")
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id.to_string()))
            .collect()
    }

    /// Every id the QML asks for is an entry of `ui.ftl`, and every entry is asked for by some QML.
    #[test]
    fn the_qml_words_and_the_ui_entries_are_the_same_set() {
        let used = ids_in_qml();
        let known = entries();
        assert!(!used.is_empty(), "no words.text or words.phrase call found: is the QML folder right?");
        let missing: Vec<&String> = used.difference(&known).collect();
        let unused: Vec<&String> = known.difference(&used).collect();
        assert!(missing.is_empty(), "QML asks for ids that ui.ftl does not name: {missing:?}");
        assert!(unused.is_empty(), "ui.ftl names ids no QML asks for: {unused:?}");
    }
}
