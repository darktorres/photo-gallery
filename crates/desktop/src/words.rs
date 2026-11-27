//! The words QML shows that it can't make itself, from `pg_wording`: the rating symbols, the timecodes, and the
//! front end's own literals (the labels and tooltips, by their `ui.ftl` id). One object per component that needs them,
//! made inside it (as the other facades are).

use crate::bridges::words_bridge::qobject;

use cxx_qt_lib::QString;

impl qobject::Words {
    pub(crate) fn rating_symbol(&self, rating: i32) -> QString {
        let rating = u8::try_from(rating.clamp(0, 5)).unwrap_or(0);
        QString::from(&pg_wording::rating_symbol(rating).to_string())
    }

    pub(crate) fn timecode(&self, seconds: i32) -> QString {
        QString::from(&pg_wording::timecode(u64::try_from(seconds).unwrap_or(0)))
    }

    pub(crate) fn count(&self, n: i32) -> QString {
        QString::from(&pg_wording::group_digits(u64::try_from(n).unwrap_or(0)))
    }

    pub(crate) fn text(&self, id: &QString) -> QString {
        QString::from(&pg_wording::ui(&String::from(id), 0.0, "", ""))
    }

    /// The entry `id` with its values: `count` is the number its plural selector reads (`$count ->`), and an entry with no
    /// selector takes 0 there, since the count is then not read. `value` fills `$text`, `second` fills `$other`.
    pub(crate) fn phrase(&self, id: &QString, count: f64, value: &QString, second: &QString) -> QString {
        QString::from(&pg_wording::ui(&String::from(id), count, &String::from(value), &String::from(second)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    /// The ids the QML asks the facade for: the first argument of each `words.text(` and `words.phrase(` call.
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
                    // A match's end is a character boundary, so `get` is `Some`.
                    let rest = source.get(at + call.len()..).unwrap_or_default();
                    let id = rest.split('"').next().unwrap_or_default();
                    ids.insert(id.to_string());
                }
            }
        }
        ids
    }

    /// The entries of `ui.ftl`: the lines that start an entry (not a comment, a continuation or a variant).
    /// The ids the native front end asks for: `pg_wording::ui("id"` in the desktop's Rust sources, and `words.text("id"` in its
    /// C++ (the reload window's QML is written there).
    fn ids_in_native() -> BTreeSet<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut ids = BTreeSet::new();
        scan_for_ids(&root.join("src"), "rs", r#"pg_wording::ui(""#, &mut ids);
        scan_for_ids(&root.join("cpp"), "cpp", r#"words.text(""#, &mut ids);
        ids
    }

    /// Adds the id after each `call` in the files of `dir` (and its subfolders) with the extension `ext`.
    fn scan_for_ids(dir: &Path, ext: &str, call: &str, ids: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).expect("a source folder") {
            let path = entry.expect("a source entry").path();
            if path.is_dir() {
                scan_for_ids(&path, ext, call, ids);
                continue;
            }
            // This file names the calls in its own text (the patterns and the doc comments), so it is not scanned.
            if path.extension().is_none_or(|e| e != ext) || path.file_name().is_some_and(|n| n == "words.rs") {
                continue;
            }
            let source = fs::read_to_string(&path).expect("a readable source file");
            for (at, _) in source.match_indices(call) {
                let rest = source.get(at + call.len()..).unwrap_or_default();
                ids.insert(rest.split('"').next().unwrap_or_default().to_string());
            }
        }
    }

    fn entries() -> BTreeSet<String> {
        include_str!("../../wording/locales/en/ui.ftl")
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id.to_string()))
            .collect()
    }

    /// The QML stub cannot read `pg_wording::RATING_SYMBOLS`, so its literal list is read here and must equal the Rust list:
    /// a change to one without the other fails this test rather than drifting.
    #[test]
    fn the_stub_rating_symbols_are_the_rust_ones() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/qml/stubs/pg/gallery/Words.qml");
        let source = fs::read_to_string(&path).expect("the Words stub");
        let line = source
            .lines()
            .find(|line| line.contains("readonly property var symbols:"))
            .expect("the stub's symbols list");
        let stub: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        assert_eq!(stub, pg_wording::RATING_SYMBOLS);
    }

    /// Every id the QML asks for is an entry of `ui.ftl`, and every entry is asked for by some QML. The key hint keys and
    /// labels, their separator and the selection line are worded by Rust (`pg_wording::hint`, the status bar), which hands
    /// the front end their text, so their ids count as asked for too.
    #[test]
    fn the_qml_words_and_the_ui_entries_are_the_same_set() {
        let mut used = ids_in_qml();
        used.extend(ids_in_native());
        for hint in pg_client::keys::Hint::ALL {
            used.insert(hint.key_id().to_string());
            used.extend(hint.label_id().map(str::to_string));
        }
        used.extend([pg_wording::HINT_SEPARATOR, pg_wording::SELECTION_LINE].map(str::to_string));
        let known = entries();
        assert!(!used.is_empty(), "no words.text or words.phrase call found: is the QML folder right?");
        let missing: Vec<&String> = used.difference(&known).collect();
        let unused: Vec<&String> = known.difference(&used).collect();
        assert!(missing.is_empty(), "QML asks for ids that ui.ftl does not name: {missing:?}");
        assert!(unused.is_empty(), "ui.ftl names ids no QML asks for: {unused:?}");
    }
}
