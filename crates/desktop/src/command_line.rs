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

use std::ffi::OsString;
use std::path::Path;
use std::sync::OnceLock;

use cxx_qt_lib::{QString, QStringList, QUrl};
use pg_model::query::{Query, Sort};

#[derive(Default)]
pub struct CommandLineRust;

/// The `--name=value` arguments, as `init` read them: (name, value), in order. Other arguments are left out.
static ARGS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Read the command line's `--name=value` arguments: once, at start, from the arguments `main` read (the program's
/// name first). The application is made from the same arguments (gui_app.rs).
pub fn init(args: &[OsString]) {
    ARGS.set(parse(args)).expect("the command line is read once");
}

/// The `--name=value` arguments among `args`, the program's name first. Windows: lossy UTF-8, as Qt reads them.
fn parse(args: &[OsString]) -> Vec<(String, String)> {
    args.iter()
        .skip(1)
        .filter_map(|a| {
            let a = a.to_string_lossy();
            let (name, value) = a.strip_prefix("--")?.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

fn args() -> &'static [(String, String)] {
    ARGS.get().map_or(&[], Vec::as_slice)
}

/// The value of `--name=…` (the first, if given twice).
pub fn value(name: &str) -> Option<&'static str> {
    args().iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// What the options that shape the first list ask for (`--query`, `--sort`, `--apply-view`), read here from the same
/// arguments. A JSON value that doesn't read is the default, as it always was.
pub struct StartOptions {
    pub query: Option<Query>,
    pub sort: Option<Sort>,
    pub apply_view: Option<String>,
}

pub fn start_options() -> StartOptions {
    StartOptions {
        query: value("query").map(pg_app::views::query_from_json),
        sort: value("sort").map(pg_app::views::sort_from_json),
        apply_view: value("apply-view").map(str::to_owned),
    }
}

/// Where a Yandex search or save gets its calls. The real client, unless `--yandex-fixture=<dir>` names a recorded
/// session (`pg_app::yandex::replay`'s folder: `upload.json`, `other_sizes.html`, `similar-N.json`, `downloads.tsv`),
/// which then answers in its place and sends nothing. Only that flag selects it; the default is never the fixture,
/// because a search sends the photo to Yandex.
pub fn yandex_source() -> pg_app::yandex::Factory {
    match value("yandex-fixture") {
        Some(dir) => {
            eprintln!("yandex: --yandex-fixture: answering from the recorded session in {dir}; nothing is sent");
            pg_app::yandex::replay(Path::new(dir))
        }
        None => pg_app::yandex::client(),
    }
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
    args().iter().find(|(n, _)| n.encode_utf16().eq(name.iter().copied())).map(|(_, v)| v.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_named_arguments_after_the_program_name() {
        let args: Vec<OsString> =
            ["--not-first=1", "--view=3", "--zoom=0.5", "scan", "-x", "--a=b=c", "--flag", "--empty="]
                .map(OsString::from)
                .into();
        assert_eq!(
            parse(&args),
            [("view", "3"), ("zoom", "0.5"), ("a", "b=c"), ("empty", "")]
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .to_vec()
        );
    }
}
