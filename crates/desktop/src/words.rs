//! The words QML shows that it can't make itself, from `pg_client::text`: the rating symbols and the timecodes. One
//! object per component that needs them, made inside it (as the other facades are).

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
    }
}

use cxx_qt_lib::QString;
use pg_client::text;

#[derive(Default)]
pub struct WordsRust;

impl qobject::Words {
    fn rating_symbol(&self, rating: i32) -> QString {
        let rating = u8::try_from(rating.clamp(0, 5)).unwrap_or(0);
        QString::from(&text::rating_symbol(rating).to_string())
    }

    fn timecode(&self, seconds: i32) -> QString {
        QString::from(&text::timecode(u64::try_from(seconds).unwrap_or(0)))
    }
}
