//! The cxx-qt bridge of [`crate::command_line`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

use crate::command_line::state::CommandLineRust;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type CommandLine = super::CommandLineRust;
    }

    unsafe extern "RustQt" {
        /// Whether `--name=…` was given.
        #[qinvokable]
        fn has(self: &CommandLine, name: &QString) -> bool;

        /// The value of `--name=…`; `""` without one.
        #[qinvokable]
        fn text(self: &CommandLine, name: &QString) -> QString;

        /// The value as a number; `fallback` without one, or when it isn't one.
        #[qinvokable]
        fn number(self: &CommandLine, name: &QString, fallback: f64) -> f64;

        /// The value as keys: a character each, but for `r` and the digit after it, which are one ("r3": rate 3).
        #[qinvokable]
        fn keys(self: &CommandLine, name: &QString) -> QStringList;
    }
}
