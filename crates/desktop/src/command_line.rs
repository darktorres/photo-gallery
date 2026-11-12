//! The window's test hooks (`--name=value` arguments, listed in TestHooks.qml), read typed: the QML doesn't parse text.
//! `--screenshot` and `--wait` are read by screenshot.rs. A path option is read by `path`, exactly as given; the QML
//! never gets a path from here.

pub(crate) mod state;

use crate::command_line_bridge::qobject;

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::OnceLock;

use cxx_qt_lib::{QString, QStringList};
use pg_model::query::{Query, Sort};
use pg_model::size::RowHeight;

/// One `--name=value` argument: its name, its value as text (what the text options read; `None` when the value is not
/// valid Unicode, never a lossy copy), and the value exactly as the system gave it (what a path option reads).
struct Arg {
    name: String,
    text: Option<String>,
    exact: OsString,
}

/// The `--name=value` arguments, as `init` read them, in order. Other arguments are left out.
static ARGS: OnceLock<Vec<Arg>> = OnceLock::new();

/// Read the command line's `--name=value` arguments: once, at start, from the arguments `main` read (the program's
/// name first). The application is made from the same arguments (gui_app.rs). An error when it was read before.
pub(crate) fn init(args: &[OsString]) -> anyhow::Result<()> {
    ARGS.set(parse(args)).map_err(|_| anyhow::anyhow!("the command line was read twice"))
}

/// The `--name=value` arguments among `args`, the program's name first.
fn parse(args: &[OsString]) -> Vec<Arg> {
    args.iter().skip(1).filter_map(arg_of).collect()
}

/// `--name=value` as an [`Arg`], or `None` for an argument of another shape (a name that is not valid Unicode is not an
/// option's: the names are ASCII). Both parts are read from the argument's own units or bytes, split at the first `=`.
fn arg_of(arg: &OsString) -> Option<Arg> {
    let (name, exact) = pg_platform::args::split_at_first_equals(arg)?;
    let name = name.to_str()?.strip_prefix("--")?.to_string();
    Some(Arg { name, text: exact.to_str().map(str::to_owned), exact })
}

/// The program's `--name=value` arguments, as `init` read them (none before it).
fn args() -> &'static [Arg] {
    ARGS.get().map_or(&[], Vec::as_slice)
}

/// The value of `--name=…` as text (the first, if given twice). `None` without one, or when its value is not valid
/// Unicode ([`not_text`] names those).
pub(crate) fn value(name: &str) -> Option<&'static str> {
    args().iter().find(|a| a.name == name).and_then(|a| a.text.as_deref())
}

/// The names of the `--name=value` arguments whose value is not valid Unicode: a text option reads none from them (a
/// path option still does). The start logs them once the log is open.
pub(crate) fn not_text() -> impl Iterator<Item = &'static str> {
    args().iter().filter(|a| a.text.is_none()).map(|a| a.name.as_str())
}

/// The value of `--name=…` as a path, exactly as given (the first, if given twice). `None` without one.
pub(crate) fn path(name: &str) -> Option<&'static Path> {
    args().iter().find(|a| a.name == name).map(|a| Path::new(&a.exact))
}

/// What the options that shape the first list ask for (`--query`, `--sort`, `--apply-view`), read here from the same
/// arguments. A JSON value that doesn't read is the default, as it always was.
pub(crate) struct StartOptions {
    pub query: Option<Query>,
    pub sort: Option<Sort>,
    pub apply_view: Option<String>,
    /// The row height the window starts at (`--size`). Never saved: the prefs keep their own.
    pub size: Option<RowHeight>,
}

pub(crate) fn start_options() -> StartOptions {
    StartOptions {
        query: value("query").map(pg_app::views::query_from_json),
        sort: value("sort").map(pg_app::views::sort_from_json),
        apply_view: value("apply-view").map(str::to_owned),
        size: size_option(),
    }
}

/// `--size` as a row height. Text that is no number is logged and ignored; a number outside the grid's range is clamped
/// into it (`RowHeight::from_finite`).
fn size_option() -> Option<RowHeight> {
    let text = value("size")?;
    let size = text.trim().parse::<f64>().ok().and_then(RowHeight::from_finite);
    if size.is_none() {
        pg_log::warn("start", &format!("--size={text} is not a row height; ignored"));
    }
    size
}

/// Dev mode as the start sets it for its run, never saved: `--dev=1` on and `--dev=0` off, else `PG_DEV` (`env`: `1`
/// or `0`), else `None`, the preferences'. Another value says nothing, and the next source decides.
fn dev_seed(args: &[Arg], env: Option<&OsStr>) -> Option<bool> {
    let flag = |value: &str| match value.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };
    args.iter()
        .find(|a| a.name == "dev")
        .and_then(|a| a.text.as_deref().and_then(flag))
        .or_else(|| env.and_then(OsStr::to_str).and_then(flag))
}

/// Dev mode as this run's command line and environment seed it ([`dev_seed`]).
pub(crate) fn dev_mode_seed() -> Option<bool> {
    dev_seed(args(), std::env::var_os("PG_DEV").as_deref())
}

/// The log file the log view shows instead of the app's (`--log-view=<path>`, the screenshots' fixed log): read once,
/// with nothing followed. The app's own logging is unchanged.
pub(crate) fn log_view() -> Option<&'static Path> {
    path("log-view")
}

/// Where a Yandex search or save gets its calls. The real client, unless `--yandex-fixture=<dir>` names a recorded
/// session (`pg_app::yandex::replay`'s folder: `upload.json`, `other_sizes.html`, `similar-N.json`, `downloads.tsv`),
/// which then answers in its place and sends nothing. Only that flag selects it; the default is never the fixture,
/// because a search sends the photo to Yandex.
pub(crate) fn yandex_source() -> pg_app::yandex::YandexSource {
    match path("yandex-fixture") {
        Some(dir) => pg_app::yandex::YandexSource::Replay(dir.to_path_buf()),
        None => pg_app::yandex::YandexSource::Live,
    }
}

/// The value of `--name` as a number: `fallback` without one, or when it isn't one.
pub(crate) fn number(name: &str, fallback: f64) -> f64 {
    number_of(value(name), fallback)
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
    args().iter().find(|a| a.name.encode_utf16().eq(name.iter().copied())).and_then(|a| a.text.as_deref())
}

impl qobject::CommandLine {
    pub(crate) fn has(&self, name: &QString) -> bool {
        named(name).is_some()
    }

    pub(crate) fn text(&self, name: &QString) -> QString {
        QString::from(named(name).unwrap_or_default())
    }

    pub(crate) fn number(&self, name: &QString, fallback: f64) -> f64 {
        number_of(named(name), fallback)
    }

    pub(crate) fn keys(&self, name: &QString) -> QStringList {
        split_keys(named(name).unwrap_or_default()).iter().map(QString::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parsed arguments as (name, value) text, for the comparisons.
    fn pairs(args: &[Arg]) -> Vec<(&str, Option<&str>)> {
        args.iter().map(|a| (a.name.as_str(), a.text.as_deref())).collect()
    }

    #[test]
    fn parses_the_named_arguments_after_the_program_name() {
        let args: Vec<OsString> =
            ["--not-first=1", "--view=3", "--zoom=0.5", "scan", "-x", "--a=b=c", "--flag", "--empty="]
                .map(OsString::from)
                .into();
        assert_eq!(
            pairs(&parse(&args)),
            [("view", Some("3")), ("zoom", Some("0.5")), ("a", Some("b=c")), ("empty", Some(""))]
        );
    }

    // A value that is not valid Unicode is no text (not a lossy copy, U+FFFD in place of the odd unit), and stays exact
    // for a path option; the argument is still the option's.
    #[test]
    fn a_value_that_is_not_unicode_has_no_text() {
        let mut raw = OsString::from("--query=");
        raw.push(pg_testkit::odd_name("a"));
        let parsed = parse(&[OsString::from("program"), raw, OsString::from("--sort=name")]);
        assert_eq!(pairs(&parsed), [("query", None), ("sort", Some("name"))]);
        assert_eq!(parsed[0].exact, pg_testkit::odd_name("a"));
        let mut dev = OsString::from("--dev=");
        dev.push(pg_testkit::odd_name("1"));
        let dev = parse(&[OsString::from("program"), dev]);
        assert_eq!(dev_seed(&dev, Some(OsStr::new("0"))), Some(false), "no flag: the environment decides");
    }

    // The name is read from the argument's own units: one that is not valid Unicode names no option, rather than
    // reading as a lossy name that could match one.
    #[test]
    fn a_name_that_is_not_unicode_is_no_option() {
        let mut raw = OsString::from("--");
        raw.push(pg_testkit::odd_name("view"));
        raw.push("=3");
        assert!(parse(&[OsString::from("program"), raw]).is_empty());
    }

    // A path option keeps its exact units: a lone surrogate on Windows, a raw byte on Unix.
    #[test]
    fn a_path_argument_keeps_its_exact_bytes() {
        let mut value = pg_testkit::odd_name("a");
        value.push(".jpg");
        let mut raw = OsString::from("--find-similar=");
        raw.push(&value);
        let parsed = parse(&[OsString::from("program"), raw]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "find-similar");
        assert_eq!(parsed[0].exact, value);
    }

    /// The arguments `list`, after a program name.
    fn args_of(list: &[&str]) -> Vec<Arg> {
        let mut all = vec![OsString::from("program")];
        all.extend(list.iter().map(OsString::from));
        parse(&all)
    }

    // Dev mode is the option's for the run, else the environment's, else the preferences' (`None`); a value that is
    // neither 1 nor 0 leaves it to the next.
    #[test]
    fn dev_mode_is_seeded_by_the_option_then_the_environment() {
        let on = Some(OsStr::new("1"));
        let off = Some(OsStr::new("0"));
        assert_eq!(dev_seed(&args_of(&[]), None), None);
        assert_eq!(dev_seed(&args_of(&["--dev=1"]), None), Some(true));
        assert_eq!(dev_seed(&args_of(&["--dev=0"]), on), Some(false), "the option wins");
        assert_eq!(dev_seed(&args_of(&[]), on), Some(true));
        assert_eq!(dev_seed(&args_of(&[]), off), Some(false));
        assert_eq!(dev_seed(&args_of(&["--dev=yes"]), off), Some(false), "not a flag: the environment decides");
        assert_eq!(dev_seed(&args_of(&["--dev=yes"]), Some(OsStr::new("true"))), None);
        assert_eq!(dev_seed(&args_of(&["--dev=1", "--dev=0"]), None), Some(true), "the first is read");
        assert_eq!(dev_seed(&args_of(&["--log=1"]), None), None);
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
