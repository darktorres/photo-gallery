//! The desktop app: the window, Qt Quick over CXX-Qt. `gallery` (crates/gallery) opens it; everything below the QML lives in `pg-client` and `pg-library`.

mod alert;
mod animation;
mod app;
// The cxx-qt and cxx bridges and the state they hold, one module: cxx-qt-build takes one directory per QML module
// (src/bridges/). Its generated code writes `unsafe` and `pub` on every item, and the state structs are `pub` because
// that code names them (E0446); none of it is reachable from the crate root.
#[expect(unsafe_code, reason = "cxx-qt generates the unsafe FFI glue of each bridge")]
#[expect(
    unreachable_pub,
    reason = "cxx and cxx-qt write `pub` on the items they generate, and the state structs they name are `pub` (E0446)"
)]
mod bridges;
mod command_line;
mod controller;
mod deadline;
mod facet_choices;
mod failed_files;
mod filters;
mod folders;
mod frame_sink;
mod gallery;
mod gpu_device;
mod gui_app;
mod keymap;
mod lane_model;
mod ledgers;
mod library;
mod list_diff;
mod log_model;
mod notices;
mod panic_log;
mod picker;
mod providers;
mod pump;
#[cfg(test)]
mod qml_text;
mod rows;
mod screenshot;
mod settings;
mod shell;
mod test_hooks;
mod trace;
mod video_player;
mod video_source;
mod viewport;
mod words;
mod worker;
mod yandex;

use std::ffi::OsString;
use std::io::Write;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QQmlApplicationEngine, QQmlEngine, QString, QUrl};

// The bridges the window's own code names (app_icon, gpu_device, providers, live_reload) come in here by their new path.
// command_line_bridge, gui_app_bridge and screenshot_bridge are still named by their old paths in command_line.rs,
// gui_app.rs and screenshot.rs: this import keeps those paths until those files move to `bridges::`.
#[cfg(debug_assertions)]
use bridges::live_reload;
use bridges::{app_icon, command_line_bridge, gpu_device_bridge, gui_app_bridge, providers_bridge, screenshot_bridge};

/// Opens the app with the program's arguments (the program's name first). Returns when its window closes, with the
/// application's exit status (`QGuiApplication::exec`'s: 0, or 1 when a `--screenshot` run found no window to save;
/// 1 when the application couldn't be made), for the process to exit with.
#[must_use]
#[expect(unsafe_code, reason = "FFI: the Qt application and QML engine calls that open the window, through cxx-qt")]
pub fn run(args: &[OsString]) -> i32 {
    // Qt uses PipeWire for audio when its library is installed, and crashes when PipeWire cannot start: then the app asks
    // Qt for PulseAudio instead (pg_platform::audio). Qt reads the variable when it makes the audio devices, so it is set
    // here, first, before any thread of the app's own starts.
    // SAFETY: no other thread is running yet: this is the first thing `run` does.
    unsafe { pg_platform::audio::prefer_pulseaudio_if_pipewire_fails() };
    // The crate's and the QML module's C++ initializers. A static library keeps them only where they are called.
    cxx_qt::init_crate!(pg_desktop);
    cxx_qt::init_qml_module!("pg.gallery");
    let perf_log_failure = trace::install();
    pg_trace::pg_stage_main();
    if let Err(e) = command_line::init(args) {
        return fatal(&format!("{e:#}"));
    }
    // A screenshot run is a test run: an option it can't use fails it here, on the console, before any window is made.
    if command_line::path("screenshot").is_some() && !command_line::bad_options().is_empty() {
        return refuse_screenshot(command_line::bad_options());
    }
    // `--locale=pseudo` shows every line in the pseudo-locale (the screenshot matrix's pseudo-locale scenario).
    pg_wording::set_pseudo(command_line::hooks().pseudo);
    // Declared before the engine, so dropped after it: Qt Quick renders with the device and keeps no reference.
    let mut gpu: cxx::UniquePtr<gpu_device_bridge::ffi::GpuDevice>;
    // The data folders, from the environment: no folder to keep the data in is an error the window can show, since it
    // is shown before the log exists.
    let mut config = match pg_app::Config::from_env() {
        Ok(config) => config,
        Err(e) => return fatal(&e),
    };
    // The log is appended to, so each run's part of it starts with its session's record: the build and where the data is.
    pg_log::init(
        config.layout.log().to_path_buf(),
        &format!("{} {}, data folder {}", pg_perf::PROGRAM, env!("CARGO_PKG_VERSION"), config.layout.home().display()),
    );
    panic_log::install(config.layout.log().to_path_buf());
    // Meanwhile the catalog opens, Qt starts and the QML loads. Started once the panic hook is in (a panic on this thread
    // is logged, and a recovered one is not shown).
    let gpu_thread = gpu_device::make_in_background();
    if let Some(failure) = perf_log_failure {
        pg_log::warn("perf", &failure);
    }
    // Each option or value the window can't use is logged; it is ignored (the start is the same without it).
    for bad in command_line::bad_options() {
        pg_log::warn("args", &bad.to_string());
    }
    // Sets the thumbnail cache's cap, kept in the cache (the app's Library panel sets it too); unset, it stays.
    // A GUI program has no stderr to show a bad value on, so the reason goes to gallery.log.
    config.thumb_cap = pg_platform::env::read(pg_platform::env::Var::ThumbCapGib).and_then(|text| {
        let text = text.to_string_lossy().into_owned();
        let cap = pg_model::cache::ThumbCap::parse_gib(&text);
        if cap.is_none() {
            pg_log::warn(
                "config",
                &format!(
                    "PG_THUMB_CAP_GIB: {text} isn't a number of GiB above zero; the thumbnail cache keeps its cap"
                ),
            );
        }
        cap
    });
    config.yandex = command_line::yandex_source();
    if let Err(e) = app::open(config) {
        let message = e
            .downcast_ref::<pg_app::Error>()
            .and_then(catalog_refusal_message)
            .unwrap_or_else(|| pg_wording::Message::CatalogUnopened { error: format!("{e:#}") });
        return fatal(&pg_wording::render(&message));
    }
    // The writes the window makes off its thread (`worker`); without the thread they are made where they are asked for.
    if let Err(e) = worker::start() {
        pg_log::warn("window", &format!("the window's writes stay on the Qt thread: {e:#}"));
    }
    pg_trace::pg_stage_init_done();

    // The controls style is Fusion, imported by name in the QML (`QtQuick.Controls.Fusion`): it follows the
    // window's dark palette. The default on Windows, the native style, draws light system buttons regardless.
    let mut app = gui_app::new(args);
    app_icon::ffi::set_app_icon();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        {
            let mut qml: core::pin::Pin<&mut QQmlEngine> = engine.as_mut().upcast_pin();
            for provider in pg_client::image::Provider::ALL {
                let image_provider = providers_bridge::ffi::new_image_provider(providers::kind_of(provider));
                // SAFETY: a new provider, which the engine takes ownership of.
                unsafe { qml.as_mut().add_image_provider(&QString::from(provider.key()), image_provider) };
            }
        }
        let main_qml = QUrl::from("qrc:/qt/qml/pg/gallery/qml/Main.qml");
        #[cfg(debug_assertions)]
        let qml_dir = QString::from(concat!(env!("CARGO_MANIFEST_DIR"), "/qml"));
        #[cfg(debug_assertions)]
        live_reload::ffi::read_qml_from_source(engine.as_mut(), &main_qml, &qml_dir);
        pg_trace::pg_stage_qml_load();
        engine.as_mut().load(&main_qml);
        pg_trace::pg_stage_qml_loaded();
        // Only now: the device is made meanwhile. Qt Multimedia reads the decoding choice when the QML makes the
        // first player, which the load doesn't (the viewer's media Loader makes them with the first video).
        // Made here instead when its thread couldn't be made, or ended without one.
        gpu = gpu_thread
            .ok()
            .and_then(|thread| thread.join().ok().flatten())
            .unwrap_or_else(gpu_device_bridge::ffi::make_gpu_device);
        gpu_device_bridge::ffi::choose_video_decoding(&gpu);
        #[cfg(debug_assertions)]
        // SAFETY: `gpu` is dropped after the engine.
        unsafe {
            live_reload::ffi::reload_on_change(engine.as_mut(), &main_qml, &qml_dir, gpu.pin_mut())
        };
        // SAFETY: as above.
        unsafe { gpu_device_bridge::ffi::show_windows(engine.as_mut(), gpu.pin_mut()) };
        screenshot::schedule(engine);
    }
    let status = screenshot::finish(app.as_mut().map_or(1, |app| app.exec()));
    // The window's writes come first: the session and the preferences its close saves are written, then the background
    // work stops (the reader, then the library). Then the engine ends the image providers, and Qt joins their thread
    // pools as they end: the connections those threads hold close with them, before the app closes its own
    // (`app::shutdown`).
    worker::stop();
    app::stop();
    drop(engine);
    app::shutdown();
    status
}

/// A `--screenshot` run that was given an option it can't use: each is named on stderr, and the run ends with status 2.
/// Not `fatal`: a test run has a console, and a message box would wait for a person who isn't there.
fn refuse_screenshot(bad: &[command_line::BadOption]) -> i32 {
    refusal(bad, &mut std::io::stderr())
}

/// The refusal's lines, one per bad option, written to `out`, and the status it ends with (2). Apart from the write,
/// pure: the test reads what it wrote.
fn refusal(bad: &[command_line::BadOption], out: &mut impl Write) -> i32 {
    for bad in bad {
        // A console that can't take the line is no reason to run the window: the status says the run was refused anyway.
        let _ = writeln!(out, "screenshot: refused: {bad}");
    }
    2
}

/// The box's message for a catalog file that was refused: worded by its kind, and the file is left as it was. `None` for
/// any other failure to open the catalog, which keeps its own text.
fn catalog_refusal_message(e: &pg_app::Error) -> Option<pg_wording::Message> {
    use pg_model::refusal::CatalogRefusal;
    use pg_wording::Message;
    if let pg_app::Error::FolderMissing(folder) = e {
        return Some(Message::CatalogFolderMissing { path: folder.display().to_string() });
    }
    let (path, refusal) = e.catalog_refusal()?;
    let path = path.to_string();
    Some(match refusal {
        CatalogRefusal::NewerSchema { found, current, written_by } => Message::CatalogNewerSchema {
            path,
            found: found.to_string(),
            current: current.to_string(),
            built_by: written_by
                .as_deref()
                .filter(|b| !b.is_empty())
                .map(|b| format!(" by build {b}"))
                .unwrap_or_default(),
        },
        CatalogRefusal::Drift { found, why } => {
            Message::CatalogDrift { path, found: found.to_string(), detail: why.clone() }
        }
        CatalogRefusal::PredatesKeyVersion => Message::CatalogPredatesKeyVersion { path },
        CatalogRefusal::PathRules { detail, wanted } => {
            Message::CatalogPathRules { path, detail: detail.clone(), wanted: wanted.to_string() }
        }
        CatalogRefusal::OlderSchema { found, current } => {
            Message::CatalogOlderSchema { path, found: found.to_string(), current: current.to_string() }
        }
        CatalogRefusal::NewerMark { mark, found, current, written_by } => Message::CatalogNewerMark {
            path,
            mark: mark.to_string(),
            found: found.to_string(),
            current: current.to_string(),
            built_by: written_by
                .as_deref()
                .filter(|b| !b.is_empty())
                .map(|b| format!(" by build {b}"))
                .unwrap_or_default(),
        },
        CatalogRefusal::InUse => Message::CatalogInUse { path },
        CatalogRefusal::PathTooLong => Message::CatalogPathTooLong { path },
        CatalogRefusal::ReadOnly => Message::CatalogReadOnly { path },
    })
}

/// A GUI program has nowhere to print: log the error, show it, then quit. The log comes first, so the record is there
/// even when the box is missed. Before the log has its file (a bad command line, no data folder) the record is held
/// in memory and lost with the process; the box is then the only record.
/// The status the caller returns: `run` returns it, so the destructors run (the log, the GPU thread), and the heap
/// profile is written, where `process::exit` would have skipped them.
fn fatal(message: &str) -> i32 {
    // Logged and printed: `error_box` is a no-op off Windows, so the message would otherwise be lost there.
    pg_log::error("startup", message);
    // A console that can't take the line is no reason to skip the error box below.
    let _ = writeln!(std::io::stderr(), "{message}");
    alert::error_box(message);
    1
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    use command_line::BadOption;

    /// A refused screenshot run names each bad option on its own line, and ends with status 2, the code a test run
    /// reads as refused (not the window's 0 or 1).
    #[test]
    fn a_refused_screenshot_run_names_each_bad_option_and_exits_2() {
        let bad = [
            BadOption { option: "--bogus".into(), reason: "is not an option the window reads".into() },
            BadOption { option: "--size=tall".into(), reason: "not a size".into() },
        ];
        let mut out = Vec::new();
        assert_eq!(refusal(&bad, &mut out), 2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "screenshot: refused: --bogus: is not an option the window reads; ignored\n\
             screenshot: refused: --size=tall: not a size; ignored\n"
        );
    }

    /// With no bad options there is nothing to name, so nothing is written.
    #[test]
    fn a_refusal_of_nothing_writes_nothing() {
        let mut out = Vec::new();
        refusal(&[], &mut out);
        assert!(out.is_empty());
    }
}
