//! The cxx-qt bridge of [`crate::words`]: the declarations alone, so that the allow the macro's output needs is
//! on the `bridges` module (`lib.rs`), not on anything written here.

use crate::bridges::words_state::WordsRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Words = super::WordsRust;
    }

    unsafe extern "RustQt" {
        /// The symbol of a rating (0 to 5; out of range is clamped).
        #[qinvokable]
        #[cxx_name = "ratingSymbol"]
        fn rating_symbol(self: &Words, rating: i32) -> QString;

        /// A time in whole seconds: m:ss, or h:mm:ss from an hour.
        #[qinvokable]
        #[cxx_name = "timecode"]
        fn timecode(self: &Words, seconds: i32) -> QString;

        /// A count with its thousands grouped (1,234,567): the one formatter every count on screen goes through. A
        /// negative count shows as 0.
        #[qinvokable]
        #[cxx_name = "count"]
        fn count(self: &Words, n: i32) -> QString;

        /// The words of the literal `id` (an entry of `ui.ftl`, which starts with `ui-`).
        #[qinvokable]
        #[cxx_name = "text"]
        fn text(self: &Words, id: &QString) -> QString;

        /// The words of the literal `id`, which chooses its plural form by `count` and shows `value` and `second`
        /// (the entry names them `$text` and `$other`; a number is passed as text).
        #[qinvokable]
        #[cxx_name = "phrase"]
        fn phrase(self: &Words, id: &QString, count: f64, value: &QString, second: &QString) -> QString;
    }
}
