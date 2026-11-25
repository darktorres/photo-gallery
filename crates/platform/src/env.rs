//! The environment: the one place the app reads its variables, and the registry of the `PG_*` ones.
//!
//! A variable is read through one of the readers here, so each has one rule for an empty value (unset) and one spelling
//! set for a flag. The readers take the lookup as a parameter, [`process`] being the process's own, so a test gives its
//! own. [`VARS`] lists every registered variable with what reads it: `tools/test_ci_files.py` checks the sources against
//! it, and `.env.example` documents the ones an operator sets.

#![expect(unsafe_code, reason = "the one environment write, `set_process`, which only the startup of the app may call")]

use std::ffi::OsString;
use std::path::PathBuf;

/// A variable the app reads in Rust. Its name is [`Var::name`]; its row in [`VARS`] says what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Var {
    /// The data folder (`pg_library::layout`).
    Home,
    /// The catalog folder (`pg_library::layout`).
    CatalogDir,
    /// The preferences file (`pg_library::layout`).
    Prefs,
    /// The ExifTool program (`pg_exiftool::Program`).
    Exiftool,
    /// The perf log (the desktop's trace, and `pg-driver`).
    PerfLog,
    /// The thumbnail cache's cap in GiB (the desktop's start).
    ThumbCapGib,
    /// The stage timings on stderr (the desktop's trace).
    Trace,
    /// The developer options (the command line).
    Dev,
    /// The file the desktop records its replay to.
    Record,
    /// The backtraces the heap profiler keeps untrimmed (the gallery binary).
    DhatUntrimmed,
    /// Whether a test that needs an external tool fails when it is missing (`pg_platform::testing::require_tool`).
    RequireTools,
    /// The scratch folder the tests write under (`pg_platform::testing::base`).
    TestDir,
}

impl Var {
    /// The variable's name in the environment.
    pub const fn name(self) -> &'static str {
        match self {
            Var::Home => "PG_HOME",
            Var::CatalogDir => "PG_CATALOG_DIR",
            Var::Prefs => "PG_PREFS",
            Var::Exiftool => "PG_EXIFTOOL",
            Var::PerfLog => "PG_PERF_LOG",
            Var::ThumbCapGib => "PG_THUMB_CAP_GIB",
            Var::Trace => "PG_TRACE",
            Var::Dev => "PG_DEV",
            Var::Record => "PG_RECORD",
            Var::DhatUntrimmed => "PG_DHAT_UNTRIMMED",
            Var::RequireTools => "PG_REQUIRE_TOOLS",
            Var::TestDir => "PG_TEST_DIR",
        }
    }
}

/// What a registered variable holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A file or folder.
    Path,
    /// On or off (the spellings of [`parse`]).
    Flag,
    /// A number.
    Number,
    /// Free text, read by a build script or a shader.
    Text,
}

/// Who reads a registered variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// The app, through a reader in this module or in a crate that depends on it.
    App,
    /// A build script, which reads it while the crate is built.
    Build,
    /// Native code (the C++ of the desktop app, or Qt's and the drivers' own names): read through its own parser.
    Native,
}

/// One registered variable: its name, what it holds, who reads it, and what it does, in a sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The variable's name.
    pub name: &'static str,
    /// What the value holds: a path, a flag, a number.
    pub kind: Kind,
    /// Who reads the variable.
    pub reader: Reader,
    /// What it does, in a sentence.
    pub doc: &'static str,
}

/// Every `PG_*` variable the app and its build read, one row each. A test or a probe's own input is not here: the
/// registry check (`tools/test_ci_files.py`) lists those with their reasons. Each name is a [`Var`]'s name or one a native
/// reader or a build script reads.
pub const VARS: &[Spec] = &[
    Spec {
        name: "PG_HOME",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "The data folder; the catalog, the preferences and the log are under it.",
    },
    Spec {
        name: "PG_CATALOG_DIR",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "The catalog folder, instead of the data folder's `catalog`. The command line's `--catalog-dir` wins over it.",
    },
    Spec {
        name: "PG_PREFS",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "The preferences file, instead of `gallery.json` in the data folder.",
    },
    Spec {
        name: "PG_EXIFTOOL",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "The ExifTool program; `exiftool` on PATH when unset.",
    },
    Spec {
        name: "PG_PERF_LOG",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "A file the perf log is appended to; `-` echoes each step to stderr instead.",
    },
    Spec {
        name: "PG_THUMB_CAP_GIB",
        kind: Kind::Number,
        reader: Reader::App,
        doc: "The thumbnail cache's size limit in GiB, above zero; unset keeps the cache's own cap.",
    },
    Spec {
        name: "PG_TRACE",
        kind: Kind::Flag,
        reader: Reader::App,
        doc: "On: each stage's milliseconds are printed on stderr as it is reached.",
    },
    Spec {
        name: "PG_DEV",
        kind: Kind::Flag,
        reader: Reader::App,
        doc: "On: the developer options of the window are shown (the same as `--dev=1`).",
    },
    Spec {
        name: "PG_RECORD",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "A file the desktop writes its controller's replay to, one record per line.",
    },
    Spec {
        name: "PG_DHAT_UNTRIMMED",
        kind: Kind::Flag,
        reader: Reader::App,
        doc: "Set to any value, the heap profiler keeps its backtraces whole (the `dhat-heap` build only).",
    },
    Spec {
        name: "PG_REQUIRE_TOOLS",
        kind: Kind::Flag,
        reader: Reader::App,
        doc: "On (CI sets it): a test that needs a missing external tool fails instead of skipping.",
    },
    Spec {
        name: "PG_TEST_DIR",
        kind: Kind::Path,
        reader: Reader::App,
        doc: "The folder the tests write their scratch files under, instead of the temp folder's.",
    },
    Spec {
        name: "PG_FORCE_AV1",
        kind: Kind::Flag,
        reader: Reader::Native,
        doc: "On: the desktop's D3D11 device decodes AV1 even when the adapter does not report it.",
    },
    Spec {
        name: "PG_SHARED_DEVICE",
        kind: Kind::Flag,
        reader: Reader::Native,
        doc: "Off: Qt Quick makes its own D3D11 device instead of sharing the app's.",
    },
    Spec {
        name: "PG_DESKTOP_CONTENT_KEY",
        kind: Kind::Text,
        reader: Reader::Build,
        doc: "Non-empty (CI sets it): the desktop build emits no path lines for its sources.",
    },
    Spec {
        name: "PG_VIDEO_CONTENT_KEY",
        kind: Kind::Text,
        reader: Reader::Build,
        doc: "Non-empty (CI sets it): the video build does not watch the include folder.",
    },
    Spec {
        name: "PG_FFMPEG_BIN",
        kind: Kind::Path,
        reader: Reader::Build,
        doc: "The FFmpeg binaries folder the video crate's build emits for its loader; set by the build, not by an operator.",
    },
    Spec {
        name: "PG_QT_DIR",
        kind: Kind::Path,
        reader: Reader::Build,
        doc: "The Qt kit folder the desktop build links against (the operator sets it in `.env`).",
    },
    Spec {
        name: "PG_QT_STATIC",
        kind: Kind::Flag,
        reader: Reader::Build,
        doc: "Set in the build's environment to link Qt statically from `PG_QT_DIR` (`crates/desktop/build.rs`); presence, not a value.",
    },
    Spec {
        name: "PG_FFMPEG_STATIC",
        kind: Kind::Path,
        reader: Reader::Build,
        doc: "The static FFmpeg's lib folder, read by the static Qt app's build and by a static video build (`crates/video/build.rs`).",
    },
    Spec {
        name: "PG_BENCH_DATA",
        kind: Kind::Path,
        reader: Reader::Build,
        doc: "The data folder the benchmarks compile in (`crates/bench`), set by `tools/profiling/bench.py` in the Linux container, to the copy of `PG_BENCH_DATA_SOURCE`.",
    },
    Spec {
        name: "PG_BENCH_JPEG",
        kind: Kind::Path,
        reader: Reader::Build,
        doc: "The JPEG the benchmarks compile in, instead of the default sample (`crates/bench`).",
    },
];

/// The process's value of `name`, with an empty value read as unset: the lookup the app passes where a crate takes one.
pub fn process(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// Sets `name` in the process's environment to `value`, for the native libraries that read it when they start.
///
/// # Safety
/// No other thread may be running, nor reading or writing the environment, while this runs: the caller is the first code
/// of the app, before its threads start.
pub(crate) unsafe fn set_process(name: &str, value: &str) {
    // SAFETY: the caller's contract above: no other thread is running.
    unsafe { std::env::set_var(name, value) };
}

/// The value of `var` in the environment `lookup` gives, with an empty value read as unset.
pub fn value(lookup: impl Fn(&str) -> Option<OsString>, var: Var) -> Option<OsString> {
    lookup(var.name()).filter(|value| !value.is_empty())
}

/// The value of `var` in the process's environment, with an empty value read as unset.
pub fn read(var: Var) -> Option<OsString> {
    value(process, var)
}

/// `var`'s value as a path, when it has one (see [`value`]).
pub fn path(lookup: impl Fn(&str) -> Option<OsString>, var: Var) -> Option<PathBuf> {
    value(lookup, var).map(PathBuf::from)
}

/// What a flag variable holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Unset, or empty.
    Unset,
    /// One of the on spellings (see [`parse`]).
    On,
    /// One of the off spellings.
    Off,
    /// A value that is neither, with its text, so the caller can say what it read.
    Invalid(String),
}

/// `var` as a flag, in the environment `lookup` gives: the on and off spellings of [`parse`], and anything else
/// [`Parsed::Invalid`] so the caller can say so.
pub fn flag_in(lookup: impl Fn(&str) -> Option<OsString>, var: Var) -> Parsed {
    let Some(text) = value(lookup, var) else { return Parsed::Unset };
    let lossy = text.to_string_lossy();
    match text.to_str().and_then(parse) {
        Some(true) => Parsed::On,
        Some(false) => Parsed::Off,
        None => Parsed::Invalid(lossy.into_owned()),
    }
}

/// `var` as a flag, in the process's environment (see [`flag_in`]).
pub fn flag(var: Var) -> Parsed {
    flag_in(process, var)
}

/// The on/off value of a flag's text: `1`, `true`, `yes` or `on` is `Some(true)`, `0`, `false`, `no` or `off` is
/// `Some(false)`, and anything else is `None`. The match ignores case and surrounding spaces. The C++ desktop reads its
/// flags with the same spellings (`crates/desktop/cpp/gpu_device.cpp`), and a test holds the two to one set.
pub fn parse(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let map: Vec<(String, OsString)> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |name| map.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    // Each on and off spelling reads as its value, in any case and with spaces around it; anything else reads as
    // `None`, so a misspelt flag is not taken as either.
    #[test]
    fn a_flag_reads_its_on_and_off_spellings() {
        for on in ["1", "true", "TRUE", " Yes ", "on", "On"] {
            assert_eq!(parse(on), Some(true), "{on:?}");
        }
        for off in ["0", "false", "No", "OFF", "off "] {
            assert_eq!(parse(off), Some(false), "{off:?}");
        }
        for other in ["", "maybe", "2", "enabled", "y"] {
            assert_eq!(parse(other), None, "{other:?}");
        }
    }

    // An empty value is unset for every reader: `PG_DEV=` and `PG_HOME=` mean nothing was said.
    #[test]
    fn an_empty_value_is_unset_for_every_reader() {
        let blank = env(&[("PG_HOME", ""), ("PG_TRACE", ""), ("PG_DEV", "")]);
        assert_eq!(value(&blank, Var::Home), None);
        assert_eq!(flag_in(&blank, Var::Trace), Parsed::Unset);
        assert_eq!(flag_in(&blank, Var::Dev), Parsed::Unset);
    }

    // A flag says on, off, or that its value is not one of them; the caller reports the last.
    #[test]
    fn a_flag_says_on_off_or_invalid() {
        assert_eq!(flag_in(env(&[("PG_TRACE", "1")]), Var::Trace), Parsed::On);
        assert_eq!(flag_in(env(&[("PG_TRACE", "off")]), Var::Trace), Parsed::Off);
        assert_eq!(flag_in(env(&[("PG_TRACE", "maybe")]), Var::Trace), Parsed::Invalid("maybe".into()));
        assert_eq!(flag_in(env(&[]), Var::Trace), Parsed::Unset);
    }

    // Every variable a typed reader reads is registered, once, under the same name.
    #[test]
    fn every_var_is_registered_once() {
        let vars = [
            Var::Home,
            Var::CatalogDir,
            Var::Prefs,
            Var::Exiftool,
            Var::PerfLog,
            Var::ThumbCapGib,
            Var::Trace,
            Var::Dev,
            Var::Record,
            Var::DhatUntrimmed,
            Var::RequireTools,
            Var::TestDir,
        ];
        let names: BTreeSet<&str> = VARS.iter().map(|spec| spec.name).collect();
        assert_eq!(names.len(), VARS.len(), "a name is registered twice");
        for var in vars {
            assert!(names.contains(var.name()), "{} is not in VARS", var.name());
        }
    }
}
