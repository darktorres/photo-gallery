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

use std::sync::LazyLock;

use cxx_qt_lib::{QString, QStringList, QUrl};

#[derive(Default)]
pub struct CommandLineRust;

/// The `--name=value` arguments, read once: (name, value), in order. Other arguments are left out.
static ARGS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    std::env::args()
        .skip(1)
        .filter_map(|a| {
            let (name, value) = a.strip_prefix("--")?.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
});

/// The value of `--name=…` (the first, if given twice).
pub fn value(name: &str) -> Option<&'static str> {
    ARGS.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
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

/// The value of `--name=…`, the name as QML gives it (compared as UTF-16, not converted).
fn named(name: &QString) -> Option<&'static str> {
    let name = name.as_slice();
    ARGS.iter().find(|(n, _)| n.encode_utf16().eq(name.iter().copied())).map(|(_, v)| v.as_str())
}

impl qobject::CommandLine {
    fn has(&self, name: &QString) -> bool {
        named(name).is_some()
    }

    fn text(&self, name: &QString) -> QString {
        QString::from(named(name).unwrap_or_default())
    }

    fn number(&self, name: &QString, fallback: f64) -> f64 {
        named(name).and_then(|v| v.trim().parse().ok()).unwrap_or(fallback)
    }

    fn url(&self, name: &QString) -> QUrl {
        QUrl::from_local_file(&self.text(name))
    }

    fn keys(&self, name: &QString) -> QStringList {
        split_keys(named(name).unwrap_or_default()).iter().map(QString::from).collect()
    }
}
