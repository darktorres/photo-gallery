//! The window's test hooks (`--name=value` arguments, listed in TestHooks.qml), read typed: the QML doesn't parse text.
//! `--screenshot` and `--wait` are read by screenshot.rs. A path option is read by `path`, exactly as given; the QML
//! never gets a path from here.

mod state;
use state::CommandLineRust;

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

use std::ffi::OsString;
use std::path::Path;
use std::sync::OnceLock;

use cxx_qt_lib::{QString, QStringList};
use pg_model::query::{Query, Sort};

/// One `--name=value` argument: its name, its value as text (lossy: what the text options read), and the value exactly
/// as the system gave it (what a path option reads).
struct Arg {
    name: String,
    value: String,
    exact: OsString,
}

/// The `--name=value` arguments, as `init` read them, in order. Other arguments are left out.
static ARGS: OnceLock<Vec<Arg>> = OnceLock::new();

/// Read the command line's `--name=value` arguments: once, at start, from the arguments `main` read (the program's
/// name first). The application is made from the same arguments (gui_app.rs). An error when it was read before.
pub fn init(args: &[OsString]) -> anyhow::Result<()> {
    ARGS.set(parse(args)).map_err(|_| anyhow::anyhow!("the command line was read twice"))
}

/// The `--name=value` arguments among `args`, the program's name first.
fn parse(args: &[OsString]) -> Vec<Arg> {
    args.iter().skip(1).filter_map(arg_of).collect()
}

/// `--name=value` as an [`Arg`], or `None` for an argument of another shape. The name is text (names are ASCII); the
/// value is the argument's own units or bytes after the first `=`.
fn arg_of(arg: &OsString) -> Option<Arg> {
    let text = arg.to_string_lossy();
    let (name, value) = text.strip_prefix("--")?.split_once('=')?;
    Some(Arg { name: name.to_string(), value: value.to_string(), exact: pg_platform::names::after_first_equals(arg)? })
}

/// The part of `arg` after its first `=`, as the system holds it.
fn args() -> &'static [Arg] {
    ARGS.get().map_or(&[], Vec::as_slice)
}

/// The value of `--name=…` (the first, if given twice).
pub fn value(name: &str) -> Option<&'static str> {
    args().iter().find(|a| a.name == name).map(|a| a.value.as_str())
}

/// The value of `--name=…` as a path, exactly as given (the first, if given twice). `None` without one.
pub fn path(name: &str) -> Option<&'static Path> {
    args().iter().find(|a| a.name == name).map(|a| Path::new(&a.exact))
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
    match path("yandex-fixture") {
        Some(dir) => {
            crate::log::note(&format!(
                "yandex: --yandex-fixture: answering from the recorded session in {}; nothing is sent",
                dir.display()
            ));
            pg_app::yandex::replay(dir)
        }
        None => pg_app::yandex::client(),
    }
}

/// The value as a number: `fallback` without one, or when it isn't one.
fn number_of(value: Option<&str>, fallback: f64) -> f64 {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(fallback)
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
    args().iter().find(|a| a.name.encode_utf16().eq(name.iter().copied())).map(|a| a.value.as_str())
}

impl qobject::CommandLine {
    fn has(&self, name: &QString) -> bool {
        named(name).is_some()
    }

    fn text(&self, name: &QString) -> QString {
        QString::from(named(name).unwrap_or_default())
    }

    fn number(&self, name: &QString, fallback: f64) -> f64 {
        number_of(named(name), fallback)
    }

    fn keys(&self, name: &QString) -> QStringList {
        split_keys(named(name).unwrap_or_default()).iter().map(QString::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parsed arguments as (name, value) text, for the comparisons.
    fn pairs(args: &[Arg]) -> Vec<(&str, &str)> {
        args.iter().map(|a| (a.name.as_str(), a.value.as_str())).collect()
    }

    #[test]
    fn parses_the_named_arguments_after_the_program_name() {
        let args: Vec<OsString> =
            ["--not-first=1", "--view=3", "--zoom=0.5", "scan", "-x", "--a=b=c", "--flag", "--empty="]
                .map(OsString::from)
                .into();
        assert_eq!(pairs(&parse(&args)), [("view", "3"), ("zoom", "0.5"), ("a", "b=c"), ("empty", "")]);
    }

    // A path option keeps its exact units: a lone surrogate on Windows, a raw byte on Unix.
    #[test]
    fn a_path_argument_keeps_its_exact_bytes() {
        #[cfg(windows)]
        let value = {
            use std::os::windows::ffi::OsStringExt;
            let mut units: Vec<u16> = r"D:\a".encode_utf16().collect();
            units.push(0xD800);
            units.extend("b.jpg".encode_utf16());
            OsString::from_wide(&units)
        };
        #[cfg(unix)]
        let value = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"/a/\xFF.jpg".to_vec())
        };
        let mut raw = OsString::from("--find-similar=");
        raw.push(&value);
        let parsed = parse(&[OsString::from("program"), raw]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "find-similar");
        assert_eq!(parsed[0].exact, value);
    }

    // The numeric options read as they always did: the number, trimmed; the fallback otherwise.
    #[test]
    fn a_number_option_is_its_number_or_the_fallback() {
        assert_eq!(number_of(Some(" 0.5 "), 1.0), 0.5);
        assert_eq!(number_of(Some("3"), 1.0), 3.0);
        assert_eq!(number_of(Some("x"), 1.0), 1.0);
        assert_eq!(number_of(Some(""), 1.0), 1.0);
        assert_eq!(number_of(None, 1.0), 1.0);
    }
}
