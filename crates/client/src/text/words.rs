//! The words: the English Fluent files, the bundle each thread reads them into, and the pseudo-locale. A line is
//! looked up here by its id ([`shown`]), with its values already set.

use std::sync::atomic::{AtomicBool, Ordering};

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

/// The English words, one entry per message (see the file's header).
pub(super) const ENGLISH: &str = include_str!("../../locales/en/text.ftl");

/// The English words of the front end's own literals, by id (`ui`): the labels, tooltips and prompts the QML shows.
const ENGLISH_UI: &str = include_str!("../../locales/en/ui.ftl");

thread_local! {
    // Fluent's bundle is not thread-safe, so each thread that words a line builds its own from the files once.
    static BUNDLE: FluentBundle<FluentResource> = english();
}

/// Set by `--locale=pseudo`: every line is then shown in the pseudo-locale (see [`pseudo`]).
static PSEUDO: AtomicBool = AtomicBool::new(false);

/// Shows the lines in the pseudo-locale when `on`, or in English.
pub fn set_pseudo(on: bool) {
    PSEUDO.store(on, Ordering::Relaxed);
}

fn english() -> FluentBundle<FluentResource> {
    // A literal tag, so it parses; the root locale would stand in for it otherwise.
    let langid: LanguageIdentifier = "en".parse().unwrap_or_default();
    let mut bundle = FluentBundle::new(vec![langid]);
    // Fluent wraps each placeable in invisible isolation marks for right-to-left text; the screen wants the plain text.
    bundle.set_use_isolating(false);
    for source in [ENGLISH, ENGLISH_UI] {
        let resource = FluentResource::try_new(source.to_string()).unwrap_or_else(|(resource, _)| resource);
        // The English entries name no id twice (the tests check it); were one named twice, its first entry is kept
        // and the rest of the file is added all the same.
        let _ = bundle.add_resource(resource);
    }
    bundle
}

/// A line of the front end's own literals: the entry `id` of `locales/en/ui.ftl`, with `count` (which chooses a plural
/// form, and is not shown) and the values `text` and `other`, which are shown as they are, so a number goes in as text
/// (Fluent may group its digits). A missing entry shows its id, which the desktop's test refuses.
pub fn ui(id: &str, count: f64, text: &str, other: &str) -> String {
    let mut args = FluentArgs::new();
    args.set("count", count);
    args.set("text", text);
    args.set("other", other);
    shown(id, &args)
}

/// The entry `id` worded with `args`, in English or in the pseudo-locale; the id itself when there is no entry.
pub(super) fn shown(id: &str, args: &FluentArgs<'_>) -> String {
    let line = BUNDLE.with(|bundle| {
        let Some(entry) = bundle.get_message(id) else { return id.to_string() };
        let Some(pattern) = entry.value() else { return id.to_string() };
        let mut errors = Vec::new();
        bundle.format_pattern(pattern, Some(args), &mut errors).into_owned()
    });
    if PSEUDO.load(Ordering::Relaxed) { pseudo(&line) } else { line }
}

/// The pseudo-locale: each letter accented, the line padded by a third and bracketed, so that a place too small for a
/// longer translation shows as clipped in the screenshot matrix. Digits and symbols are kept.
fn pseudo(english: &str) -> String {
    const ACCENTED: &str = "åƀçđéƒğĥíĵķĺḿñöþqŕšťüṽŵẋýž";
    let accented: Vec<char> = ACCENTED.chars().collect();
    let mut out = String::with_capacity(english.len() * 2);
    out.push('⟦');
    for c in english.chars() {
        if c.is_ascii_alphabetic() {
            let index = u8::try_from(c.to_ascii_lowercase()).unwrap_or(b'a') - b'a';
            let base = accented.get(usize::from(index)).copied().unwrap_or(c);
            out.push(if c.is_ascii_uppercase() { base.to_uppercase().next().unwrap_or(base) } else { base });
        } else {
            out.push(c);
        }
    }
    out.extend(std::iter::repeat_n('~', english.chars().count() / 3 + 1));
    out.push('⟧');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::test_support::entries_in_file;

    /// The ids of the entries in `ui.ftl`, the same way as `entries_in_file`.
    fn ui_entries() -> Vec<&'static str> {
        ENGLISH_UI
            .lines()
            .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
            .collect()
    }

    #[test]
    fn the_ui_file_parses_and_names_each_id_once() {
        assert!(FluentResource::try_new(ENGLISH_UI.to_string()).is_ok(), "ui.ftl has a syntax error");
        let mut ids = ui_entries();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "ui.ftl names an id twice");
        assert!(ids.iter().all(|id| id.starts_with("ui-")), "every ui.ftl id starts with ui-");
        assert!(entries_in_file().iter().all(|id| !id.starts_with("ui-")), "text.ftl has a ui- id");
    }

    #[test]
    fn a_ui_line_is_worded_by_its_entry_and_its_values() {
        assert_eq!(ui("ui-save-picked", 2.0, "2", ""), "Save 2 images…");
        assert_eq!(ui("ui-save-picked", 1.0, "1", ""), "Save 1 image…");
        assert_eq!(ui("ui-window-title", 0.0, "D:\\Photos", ""), "Photo Gallery — D:\\Photos");
        assert_eq!(ui("ui-no-such-entry", 0.0, "", ""), "ui-no-such-entry", "a missing entry shows its id");
    }

    #[test]
    fn the_pseudo_locale_accents_letters_and_keeps_digits() {
        let line = pseudo("Save 12 images, Ab");
        assert!(line.starts_with('⟦') && line.ends_with('⟧'), "{line}");
        assert!(line.contains("12"), "{line}");
        assert!(line.contains("Åƀ"), "the capital A is accented as Å and the lowercase b as ƀ: {line}");
        assert!(line.chars().count() > "Save 12 images, Ab".chars().count() + 4, "the pseudo line is padded: {line}");
        assert!(!line.contains("Save"), "the letters are accented: {line}");
    }
}
