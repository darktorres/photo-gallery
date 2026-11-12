//! The window's command line: the `--name=value` arguments, read once and typed. Every option the window reads is in
//! [`OPTIONS`]; another `--name` is an error, since a misspelt test hook would otherwise do nothing, silently. A value is
//! parsed into its type when the arguments are read, once. One that doesn't parse, or a name that isn't an option, is a
//! [`BadOption`]: the start logs each, and under `--screenshot` any one fails the run before the window is made.
//!
//! Two sets are read here: the start's options ([`StartOptions`], what shapes the window's first state) and the test
//! hooks ([`Hooks`], what the QML does once the first list is shown). The QML reads the hooks through the `CommandLine`
//! object's typed properties (`command_line_bridge.rs`). The path options (`--screenshot`, `--log-view`, …) are read
//! exactly as the system gave them ([`path`]), since a path need not be text.

pub(crate) mod state;

use crate::command_line_bridge::qobject;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

use cxx_qt_lib::{QString, QStringList};
use pg_model::query::{Query, Sort};
use pg_model::size::RowHeight;

/// Every option the window reads, by name (without the `--`). Another `--name` is an error.
pub(crate) const OPTIONS: &[&str] = &[
    // The start: the first list and the first state.
    "query",
    "sort",
    "apply-view",
    "size",
    "filters",
    "borders",
    "dev",
    // The test hooks: what the window does once its first list is shown.
    "select",
    "view",
    "viewer-keys",
    "grid-keys",
    "zoom",
    "menu",
    "mascot",
    "library",
    "save-view",
    "find-similar",
    "yandex-preview",
    "yandex-fixture",
    "add-folder",
    "save-crops",
    "log",
    "log-view",
    "inbox",
    "inbox-expand",
    // The screenshot, and the window's language.
    "screenshot",
    "wait",
    "locale",
];

/// Qt's own options, which Qt reads from the same arguments: the window accepts them and does not read them. The names are
/// the ones Qt documents for its applications (`-platform` takes one dash, so it is not one of these arguments).
pub(crate) const QT_OPTIONS: &[&str] = &[
    "platform",
    "platformpluginpath",
    "platformtheme",
    "plugin",
    "qmljsdebugger",
    "style",
    "reverse",
    "qwindowgeometry",
    "qwindowtitle",
    "qwindowicon",
    "session",
    "widgetcount",
    "display",
    "geometry",
];

/// An option or a value the window can't use, and why. The start logs each; under `--screenshot` any one fails the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BadOption {
    /// The option as written, with its `--` (or the environment variable's name).
    pub option: String,
    pub reason: String,
}

impl BadOption {
    fn new(option: impl Into<String>, reason: impl Into<String>) -> BadOption {
        BadOption { option: option.into(), reason: reason.into() }
    }
}

impl fmt::Display for BadOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}; ignored", self.option, self.reason)
    }
}

/// The start's options, typed. `None` where the option is not given. Nothing here is saved: the start's values hold for
/// the run.
#[derive(Clone, Debug, Default)]
pub(crate) struct StartOptions {
    pub query: Option<Query>,
    pub sort: Option<Sort>,
    pub apply_view: Option<String>,
    /// The row height the window starts at (`--size`). Never saved: the prefs keep their own.
    pub size: Option<RowHeight>,
    /// The filter bar open at the start (`--filters`).
    pub filters_open: Option<bool>,
    /// The borders shown at the start (`--borders`).
    pub show_borders: Option<bool>,
    /// Dev mode at the start (`--dev`, else `PG_DEV`).
    pub dev_mode: Option<bool>,
}

/// The test hooks, typed: what the window does once its first list is shown. The QML reads each through `CommandLine`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hooks {
    /// Select this item (`--select`).
    pub select: Option<usize>,
    /// Open this item in the viewer (`--view`).
    pub view: Option<usize>,
    /// The viewer's zoom, in device pixels per image pixel (`--zoom`), positive.
    pub zoom: Option<f64>,
    /// The mascot to show, 1 to 40 (`--mascot`); none is a random one.
    pub mascot: Option<u32>,
    /// Open the views menu (`--menu=views`).
    pub menu_views: bool,
    /// Open the library folders (`--library=1`).
    pub library: bool,
    /// Open the save-view dialog (`--save-view=1`).
    pub save_view: bool,
    /// Show this search result at full size, with `--find-similar` (`--yandex-preview`).
    pub yandex_preview: Option<usize>,
    /// Open the log view, in dev mode (`--log=1`).
    pub log: bool,
    /// Open the inbox (`--inbox=1`).
    pub inbox: bool,
    /// Open this ledger's entry in the inbox (`--inbox-expand`).
    pub inbox_expand: Option<String>,
    /// The viewer's keys, each a key or a rating `r0` to `r5` (`--viewer-keys`).
    pub viewer_keys: Vec<String>,
    /// The grid's keys, L, R, U or D (`--grid-keys`).
    pub grid_keys: Vec<String>,
    /// Show every line in the pseudo-locale (`--locale=pseudo`).
    pub pseudo: bool,
    /// The most to wait for the screenshot's window to settle, in seconds (`--wait`), 10 by default.
    pub wait: f64,
}

impl Default for Hooks {
    fn default() -> Hooks {
        Hooks {
            select: None,
            view: None,
            zoom: None,
            mascot: None,
            menu_views: false,
            library: false,
            save_view: false,
            yandex_preview: None,
            log: false,
            inbox: false,
            inbox_expand: None,
            viewer_keys: Vec::new(),
            grid_keys: Vec::new(),
            pseudo: false,
            wait: 10.0,
        }
    }
}

/// One `--name=value` argument: its name, its value as text (what the text options read; `None` when the value is not
/// valid Unicode, never a lossy copy), and the value exactly as the system gave it (what a path option reads).
struct Arg {
    name: String,
    text: Option<String>,
    exact: OsString,
}

/// Everything the arguments read to: the arguments themselves (for the path options), the start's options, the hooks, and
/// the bad options.
#[derive(Debug)]
struct Read {
    start: StartOptions,
    hooks: Hooks,
    bad: Vec<BadOption>,
}

/// The `--name=value` arguments, as `init` read them, in order. Other arguments are left out.
static ARGS: OnceLock<Vec<Arg>> = OnceLock::new();

/// What the arguments read to, as `init` read them.
static READ: OnceLock<Read> = OnceLock::new();

/// Read the command line's arguments: once, at start, from the arguments `main` read (the program's name first). The
/// application is made from the same arguments (gui_app.rs). An error when they were read before.
pub(crate) fn init(args: &[OsString]) -> anyhow::Result<()> {
    let read = read_of(args, std::env::var_os("PG_DEV").as_deref());
    ARGS.set(parse(args)).map_err(|_| anyhow::anyhow!("the command line was read twice"))?;
    READ.set(read).map_err(|_| anyhow::anyhow!("the command line was read twice"))
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

/// The arguments read to what the window uses ([`Read`]), given the program's arguments and the environment's `PG_DEV`.
fn read_of(args: &[OsString], dev_env: Option<&OsStr>) -> Read {
    let parsed = parse(args);
    let mut bad = Vec::new();
    // A `--name` with no `=` is no option's value: it is named, not silently skipped.
    for word in args.iter().skip(1) {
        let Some(word) = word.to_str().and_then(|w| w.strip_prefix("--")) else { continue };
        if !word.contains('=') {
            known_name(word, &mut bad, true);
        }
    }
    for arg in &parsed {
        known_name(&arg.name, &mut bad, false);
    }
    let start = StartOptions {
        query: typed(&parsed, "query", &mut bad, |t| {
            pg_app::views::query_from_json(t).map_err(|e| format!("does not read as a query: {e}"))
        }),
        sort: typed(&parsed, "sort", &mut bad, |t| {
            pg_app::views::sort_from_json(t).map_err(|e| format!("does not read as a sort: {e}"))
        }),
        apply_view: text_of(&parsed, "apply-view", &mut bad).map(str::to_owned),
        size: typed(&parsed, "size", &mut bad, |t| {
            number(t).and_then(|n| RowHeight::from_finite(n).ok_or_else(|| "is not a finite row height".to_string()))
        }),
        filters_open: typed(&parsed, "filters", &mut bad, flag),
        show_borders: typed(&parsed, "borders", &mut bad, flag),
        dev_mode: typed(&parsed, "dev", &mut bad, flag).or_else(|| env_flag(dev_env, &mut bad)),
    };
    let hooks = Hooks {
        select: typed(&parsed, "select", &mut bad, index),
        view: typed(&parsed, "view", &mut bad, index),
        zoom: typed(&parsed, "zoom", &mut bad, |t| number(t).and_then(positive)),
        mascot: typed(&parsed, "mascot", &mut bad, |t| {
            t.trim()
                .parse::<u32>()
                .ok()
                .filter(|m| (1..=40).contains(m))
                .ok_or_else(|| "is not a mascot: 1 to 40".to_string())
        }),
        menu_views: typed(&parsed, "menu", &mut bad, |t| {
            if t == "views" { Ok(()) } else { Err("is not a menu: views is the only one".to_string()) }
        })
        .is_some(),
        library: typed(&parsed, "library", &mut bad, flag).unwrap_or(false),
        save_view: typed(&parsed, "save-view", &mut bad, flag).unwrap_or(false),
        yandex_preview: typed(&parsed, "yandex-preview", &mut bad, index).filter(|_| {
            // Only a search's result can be shown at full size: without --find-similar, the option says nothing.
            has_option(&parsed, "find-similar")
        }),
        log: typed(&parsed, "log", &mut bad, flag).unwrap_or(false),
        inbox: typed(&parsed, "inbox", &mut bad, flag).unwrap_or(false),
        inbox_expand: text_of(&parsed, "inbox-expand", &mut bad).map(str::to_owned),
        viewer_keys: typed(&parsed, "viewer-keys", &mut bad, viewer_keys).unwrap_or_default(),
        grid_keys: typed(&parsed, "grid-keys", &mut bad, grid_keys).unwrap_or_default(),
        pseudo: typed(&parsed, "locale", &mut bad, |t| {
            if t == "pseudo" { Ok(()) } else { Err("is not a locale: pseudo is the only one".to_string()) }
        })
        .is_some(),
        wait: typed(&parsed, "wait", &mut bad, |t| number(t).and_then(positive)).unwrap_or(10.0),
    };
    if has_option(&parsed, "yandex-preview") && !has_option(&parsed, "find-similar") {
        bad.push(BadOption::new("--yandex-preview", "needs --find-similar; ignored"));
    }
    Read { start, hooks, bad }
}

/// Whether `name` is an option the window reads (or one of Qt's). Otherwise it is named as bad, and `bare` says the
/// argument had no value at all.
fn known_name(name: &str, bad: &mut Vec<BadOption>, bare: bool) {
    if QT_OPTIONS.contains(&name) {
        return;
    }
    if !OPTIONS.contains(&name) {
        bad.push(BadOption::new(format!("--{name}"), "is not an option the window reads"));
    } else if bare {
        bad.push(BadOption::new(format!("--{name}"), "needs a value: --name=value"));
    }
}

/// Whether `--name=…` is given at all.
fn has_option(args: &[Arg], name: &str) -> bool {
    args.iter().any(|a| a.name == name)
}

/// The text of `--name=…` (the first, if given twice). A value that is not valid Unicode is a bad option: no text.
fn text_of<'a>(args: &'a [Arg], name: &str, bad: &mut Vec<BadOption>) -> Option<&'a str> {
    let arg = args.iter().find(|a| a.name == name)?;
    if arg.text.is_none() {
        bad.push(BadOption::new(format!("--{name}"), "its value is not valid Unicode"));
    }
    arg.text.as_deref()
}

/// The value of `--name=…` read by `parse`. A value that doesn't parse is a bad option, and the option reads as not given.
fn typed<T>(
    args: &[Arg],
    name: &str,
    bad: &mut Vec<BadOption>,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<T> {
    let text = text_of(args, name, bad)?;
    match parse(text) {
        Ok(value) => Some(value),
        Err(reason) => {
            bad.push(BadOption::new(format!("--{name}={text}"), reason));
            None
        }
    }
}

/// A boolean from the environment (`PG_DEV`), read as an option's flag. Another value is bad, and reads as not given.
fn env_flag(env: Option<&OsStr>, bad: &mut Vec<BadOption>) -> Option<bool> {
    let env = env?;
    match env.to_str().map(flag) {
        Some(Ok(on)) => Some(on),
        Some(Err(reason)) => {
            bad.push(BadOption::new("PG_DEV", format!("{} {reason}", env.to_string_lossy())));
            None
        }
        None => {
            bad.push(BadOption::new("PG_DEV", "its value is not valid Unicode"));
            None
        }
    }
}

/// A boolean option: `1` or `true` is on, `0` or `false` is off (trimmed); another text is not one.
fn flag(text: &str) -> Result<bool, String> {
    match text.trim() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err("is not 1, 0, true or false".to_string()),
    }
}

/// A number, trimmed.
fn number(text: &str) -> Result<f64, String> {
    text.trim().parse::<f64>().map_err(|_| "is not a number".to_string())
}

/// A positive finite number.
fn positive(v: f64) -> Result<f64, String> {
    if v.is_finite() && v > 0.0 { Ok(v) } else { Err("is not a positive number".to_string()) }
}

/// A whole number, trimmed: an item's index.
fn index(text: &str) -> Result<usize, String> {
    text.trim().parse::<usize>().map_err(|_| "is not a whole number".to_string())
}

/// The keys of `--grid-keys`: L, R, U and D, each a key (the grid's steps).
fn grid_keys(text: &str) -> Result<Vec<String>, String> {
    text.chars()
        .map(|c| match c {
            'L' | 'R' | 'U' | 'D' => Ok(c.to_string()),
            other => Err(format!("has '{other}', which is not a grid key: L, R, U or D")),
        })
        .collect()
}

/// The keys of `--viewer-keys`: a key a character each (L, B, N, H, K, +, 1, <, >), or `r` and a rating digit after it
/// (`r3` rates 3, `r0` to `r5`).
fn viewer_keys(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            'r' => match chars.next() {
                Some(d @ '0'..='5') => out.push(format!("r{d}")),
                _ => return Err("has 'r' without a rating 0 to 5 after it".to_string()),
            },
            'L' | 'B' | 'N' | 'H' | 'K' | '+' | '1' | '<' | '>' => out.push(c.to_string()),
            other => return Err(format!("has '{other}', which is not a viewer key")),
        }
    }
    Ok(out)
}

/// The start's options, as the arguments read them (`None` for each that is not given or doesn't read).
pub(crate) fn start_options() -> StartOptions {
    read().start.clone()
}

/// The test hooks, as the arguments read them.
pub(crate) fn hooks() -> &'static Hooks {
    &read().hooks
}

/// The options and values the window can't use, as the arguments read them: the start logs each, and under `--screenshot`
/// any one fails the run.
pub(crate) fn bad_options() -> &'static [BadOption] {
    &read().bad
}

/// What the arguments read to. Before `init` there are no arguments, so the defaults.
fn read() -> &'static Read {
    READ.get_or_init(|| read_of(&[], None))
}

/// The value of `--name=…`, exactly as given (the first, if given twice). `None` without one.
pub(crate) fn path(name: &str) -> Option<&'static Path> {
    ARGS.get().and_then(|args| args.iter().find(|a| a.name == name)).map(|a| Path::new(&a.exact))
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

/// An item index as QML takes it: `none` when it is not given. An index that doesn't fit an `i32` is the largest.
fn index_for_qml(index: Option<usize>, none: i32) -> i32 {
    index.map_or(none, |i| i32::try_from(i).unwrap_or(i32::MAX))
}

/// The `CommandLine` object's typed properties, read by TestHooks.qml. Where QML has no type for "not given", the
/// property says so with a value QML checks for, documented on each: `-1` for an index, `0` for a zoom or a mascot,
/// empty for a text.
impl qobject::CommandLine {
    /// The item to select: `-1` when none.
    pub(crate) fn read_select(&self) -> i32 {
        index_for_qml(hooks().select, -1)
    }

    /// The item to open in the viewer: `-1` when none.
    pub(crate) fn read_view(&self) -> i32 {
        index_for_qml(hooks().view, -1)
    }

    /// The viewer's zoom: `0` when none (a given zoom is positive).
    pub(crate) fn read_zoom(&self) -> f64 {
        hooks().zoom.unwrap_or(0.0)
    }

    /// The mascot to show: `0` for a random one.
    pub(crate) fn read_mascot(&self) -> i32 {
        hooks().mascot.map_or(0, |m| i32::try_from(m).unwrap_or(0))
    }

    /// The Yandex result to show at full size: `-1` when none.
    pub(crate) fn read_yandex_preview(&self) -> i32 {
        index_for_qml(hooks().yandex_preview, -1)
    }

    /// The ledger whose inbox entry to open: empty when none.
    pub(crate) fn read_inbox_expand(&self) -> QString {
        QString::from(hooks().inbox_expand.as_deref().unwrap_or_default())
    }

    pub(crate) fn read_viewer_keys(&self) -> QStringList {
        hooks().viewer_keys.iter().map(QString::from).collect()
    }

    pub(crate) fn read_grid_keys(&self) -> QStringList {
        hooks().grid_keys.iter().map(QString::from).collect()
    }

    pub(crate) fn read_menu_views(&self) -> bool {
        hooks().menu_views
    }

    pub(crate) fn read_library(&self) -> bool {
        hooks().library
    }

    pub(crate) fn read_save_view(&self) -> bool {
        hooks().save_view
    }

    pub(crate) fn read_log(&self) -> bool {
        hooks().log
    }

    pub(crate) fn read_inbox(&self) -> bool {
        hooks().inbox
    }

    /// Whether `--find-similar` is given (the search runs and its panel shows).
    pub(crate) fn read_find_similar(&self) -> bool {
        path("find-similar").is_some()
    }

    /// Whether `--screenshot` is given (the window is being photographed).
    pub(crate) fn read_screenshot(&self) -> bool {
        path("screenshot").is_some()
    }

    /// Whether `--add-folder` is given (the folder picker opens with the library job).
    pub(crate) fn read_add_folder(&self) -> bool {
        path("add-folder").is_some()
    }

    /// Whether `--save-crops` is given (the save picker opens with the library job).
    pub(crate) fn read_save_crops(&self) -> bool {
        path("save-crops").is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments as the program gets them: the program's name first, then each `words` as written.
    fn args_of(words: &[&str]) -> Vec<OsString> {
        std::iter::once("program").chain(words.iter().copied()).map(OsString::from).collect()
    }

    /// The read of `words`, with `PG_DEV` as `dev_env`.
    fn read(words: &[&str], dev_env: Option<&str>) -> Read {
        read_of(&args_of(words), dev_env.map(OsStr::new))
    }

    /// The parsed arguments as (name, value) text, for the comparisons.
    fn pairs(args: &[Arg]) -> Vec<(&str, Option<&str>)> {
        args.iter().map(|a| (a.name.as_str(), a.text.as_deref())).collect()
    }

    /// The bad options' names, in order.
    fn bad_names(read: &Read) -> Vec<&str> {
        read.bad.iter().map(|b| b.option.as_str()).collect()
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

    // A value that is not valid Unicode has no text (not a lossy copy), and stays exact for a path option. As a typed
    // option it is a bad option, not a silent default.
    #[test]
    fn a_value_that_is_not_unicode_has_no_text_and_is_bad_for_a_typed_option() {
        let mut raw = OsString::from("--query=");
        raw.push(pg_testkit::odd_name("a"));
        let parsed = parse(&[OsString::from("program"), raw, OsString::from("--sort=name")]);
        assert_eq!(pairs(&parsed), [("query", None), ("sort", Some("name"))]);
        assert_eq!(parsed[0].exact, pg_testkit::odd_name("a"));
        let mut query = OsString::from("--query=");
        query.push(pg_testkit::odd_name("a"));
        let read = read_of(&[OsString::from("program"), query], None);
        assert_eq!(bad_names(&read), ["--query"]);
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

    // Dev mode is the option's for the run, else the environment's, else the preferences' (`None`); a value that is
    // neither a flag is bad, and the next source decides.
    #[test]
    fn dev_mode_is_read_by_the_option_then_the_environment() {
        assert_eq!(read(&[], None).start.dev_mode, None);
        assert_eq!(read(&["--dev=1"], None).start.dev_mode, Some(true));
        assert_eq!(read(&["--dev=0"], Some("1")).start.dev_mode, Some(false), "the option wins");
        assert_eq!(read(&[], Some("1")).start.dev_mode, Some(true));
        assert_eq!(read(&[], Some("0")).start.dev_mode, Some(false));
        assert_eq!(read(&[], Some("true")).start.dev_mode, Some(true), "true is a flag too");
        let bad = read(&["--dev=yes"], Some("0"));
        assert_eq!(bad.start.dev_mode, Some(false), "not a flag: the environment decides");
        assert_eq!(bad_names(&bad), ["--dev=yes"]);
        let env = read(&[], Some("yes"));
        assert_eq!(env.start.dev_mode, None);
        assert_eq!(bad_names(&env), ["PG_DEV"], "a bad PG_DEV is named");
        assert_eq!(read(&["--dev=1", "--dev=0"], None).start.dev_mode, Some(true), "the first is read");
    }

    #[test]
    fn a_flag_is_one_or_true_on_and_zero_or_false_off() {
        assert_eq!(flag("1"), Ok(true));
        assert_eq!(flag(" true "), Ok(true));
        assert_eq!(flag("0"), Ok(false));
        assert_eq!(flag("false"), Ok(false));
        assert!(flag("yes").is_err());
        assert!(flag("").is_err());
    }

    /// Every hook and start option reads to its type, and the QML's "not given" values are the defaults.
    #[test]
    fn the_hooks_read_typed() {
        let r = read(
            &[
                "--select=5",
                "--view=0",
                "--zoom=2.5",
                "--mascot=40",
                "--menu=views",
                "--library=1",
                "--save-view=0",
                "--log=true",
                "--inbox=1",
                "--inbox-expand=import",
                "--viewer-keys=LBr3",
                "--grid-keys=LRUD",
                "--locale=pseudo",
                "--wait=2.5",
                "--find-similar=shot.jpg",
                "--yandex-preview=1",
            ],
            None,
        );
        assert!(r.bad.is_empty(), "{:?}", r.bad);
        let h = &r.hooks;
        assert_eq!((h.select, h.view, h.zoom, h.mascot), (Some(5), Some(0), Some(2.5), Some(40)));
        assert!(h.menu_views && h.library && !h.save_view && h.log && h.inbox && h.pseudo);
        assert_eq!(h.inbox_expand.as_deref(), Some("import"));
        assert_eq!(h.viewer_keys, ["L", "B", "r3"]);
        assert_eq!(h.grid_keys, ["L", "R", "U", "D"]);
        assert_eq!((h.wait, h.yandex_preview), (2.5, Some(1)));
        let none = read(&[], None).hooks;
        assert_eq!(none, Hooks::default());
        assert_eq!(none.wait, 10.0, "the wait is 10 seconds when not given");
    }

    #[test]
    fn a_bad_hook_value_is_named_and_reads_as_not_given() {
        let r = read(
            &[
                "--select=-1",
                "--zoom=0",
                "--mascot=41",
                "--menu=tree",
                "--viewer-keys=Q",
                "--grid-keys=X",
                "--locale=fr",
            ],
            None,
        );
        assert_eq!(r.hooks.select, None);
        assert_eq!(r.hooks.zoom, None);
        assert_eq!(r.hooks.mascot, None);
        assert!(!r.hooks.menu_views && !r.hooks.pseudo);
        assert!(r.hooks.viewer_keys.is_empty() && r.hooks.grid_keys.is_empty());
        assert_eq!(
            bad_names(&r),
            [
                "--select=-1",
                "--zoom=0",
                "--mascot=41",
                "--menu=tree",
                "--viewer-keys=Q",
                "--grid-keys=X",
                "--locale=fr"
            ]
        );
    }

    #[test]
    fn an_option_the_window_does_not_read_is_named() {
        let r = read(&["--selekt=3", "--library"], None);
        // The bare words are named first, then the parsed arguments in order.
        assert_eq!(bad_names(&r), ["--library", "--selekt"]);
        assert!(r.bad[0].reason.contains("needs a value"), "{:?}", r.bad[0]);
    }

    /// Qt reads its own options from the same arguments: the window accepts each name Qt documents, and reads none.
    #[test]
    fn qts_own_options_pass_through_unread() {
        let words = [
            "--platform=offscreen",
            "--platformpluginpath=x",
            "--platformtheme=y",
            "--plugin=z",
            "--qmljsdebugger=port:1",
            "--style=Fusion",
            "--reverse",
            "--qwindowgeometry=10x10",
            "--qwindowtitle=t",
            "--qwindowicon=i",
            "--session=s",
            "--widgetcount",
            "--display=:0",
            "--geometry=1x1",
        ];
        for word in words {
            assert!(read(&[word], None).bad.is_empty(), "{word}");
        }
        assert!(QT_OPTIONS.contains(&"platform") && QT_OPTIONS.contains(&"geometry"));
    }

    #[test]
    fn yandex_preview_needs_a_search() {
        let r = read(&["--yandex-preview=1"], None);
        assert_eq!(r.hooks.yandex_preview, None);
        assert_eq!(bad_names(&r), ["--yandex-preview"]);
        assert!(read(&["--yandex-preview=1", "--find-similar=a.jpg"], None).bad.is_empty());
    }

    #[test]
    fn the_size_and_the_start_options_read_typed() {
        let r = read(
            &[
                "--size=120",
                "--filters=1",
                "--borders=0",
                "--sort={\"key\":\"name\",\"descending\":true}",
                "--apply-view=alpha",
            ],
            None,
        );
        assert!(r.bad.is_empty(), "{:?}", r.bad);
        assert_eq!(r.start.size.map(RowHeight::get), Some(120));
        assert_eq!((r.start.filters_open, r.start.show_borders), (Some(true), Some(false)));
        assert!(r.start.sort.is_some());
        assert_eq!(r.start.apply_view.as_deref(), Some("alpha"));
        let bad = read(&["--size=tall", "--query=not json", "--sort=["], None);
        // The start's options are read in their field order: query, sort, then size.
        assert_eq!(bad_names(&bad), ["--query=not json", "--sort=[", "--size=tall"]);
        assert_eq!(bad.start.size, None);
        assert!(bad.start.query.is_none() && bad.start.sort.is_none());
    }
}
