//! The window's test hooks (`--name=value` arguments, listed in Main.qml), read typed: the QML doesn't parse text.
//! `--screenshot` and `--wait` are main.rs's.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
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

        /// The value, a path, as a file URL.
        #[qinvokable]
        fn url(self: &CommandLine, name: &QString) -> QUrl;

        /// The value as keys: a character each, but for `r` and the digit after it, which are one ("r3": rate 3).
        #[qinvokable]
        fn keys(self: &CommandLine, name: &QString) -> QStringList;
    }
}

use cxx_qt_lib::{QString, QStringList, QUrl};

#[derive(Default)]
pub struct CommandLineRust;

/// The value of `--name=…`.
pub fn value(name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    std::env::args().skip(1).find_map(|a| a.strip_prefix(&prefix).map(str::to_string))
}

/// `keys` split as `CommandLine.keys` does.
fn split_keys(keys: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        if c == 'r' {
            out.extend(chars.next().map(|d| format!("r{d}")));
        } else {
            out.push(c.to_string());
        }
    }
    out
}

impl qobject::CommandLine {
    fn has(&self, name: &QString) -> bool {
        value(&name.to_string()).is_some()
    }

    fn text(&self, name: &QString) -> QString {
        QString::from(&value(&name.to_string()).unwrap_or_default())
    }

    fn number(&self, name: &QString, fallback: f64) -> f64 {
        value(&name.to_string()).and_then(|v| v.trim().parse().ok()).unwrap_or(fallback)
    }

    fn url(&self, name: &QString) -> QUrl {
        QUrl::from_local_file(&self.text(name))
    }

    fn keys(&self, name: &QString) -> QStringList {
        split_keys(&value(&name.to_string()).unwrap_or_default()).iter().map(QString::from).collect()
    }
}
