//! The desktop app (PLAN §7): the window, Qt Quick over CXX-Qt. `gallery` (crates/gallery) opens it with `run` when it's
//! given no command; everything below the QML lives in `pg-client` and `pg-library`.

mod alert;
mod animation;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod animation_bridge;
mod app;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod app_icon;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod clipboard;
mod command_line;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod command_line_bridge;
mod controller;
mod deadline;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod deadline_bridge;
mod facet_choices;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod facet_choices_bridge;
mod failed_files;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod failed_files_bridge;
mod filters;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod filters_bridge;
mod folders;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod folders_bridge;
mod frame_sink;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod frame_sink_bridge;
mod gallery;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod gallery_bridge;
mod gpu_device;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod gpu_device_bridge;
mod gui_app;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod gui_app_bridge;
mod keymap;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod keymap_bridge;
mod ledgers;
mod library;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod library_bridge;
mod list_diff;
#[cfg(debug_assertions)]
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod live_reload;
mod log_model;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod log_model_bridge;
mod notices;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod notices_bridge;
mod panic_log;
mod picker;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod picker_bridge;
mod providers;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod providers_bridge;
mod pump;
#[cfg(test)]
mod qml_text;
mod rows;
mod screenshot;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod screenshot_bridge;
mod settings;
mod settings_bridge;
mod shell;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod shell_bridge;
mod trace;
mod trace_bridge;
mod video_player;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod video_player_bridge;
mod video_source;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod video_source_bridge;
mod viewport;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod viewport_bridge;
mod words;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod words_bridge;

mod yandex;
#[expect(unreachable_pub, reason = "cxx and cxx-qt write `pub` on every item they generate for a bridge")]
mod yandex_bridge;

use std::ffi::OsString;
use std::io::Write;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QQmlApplicationEngine, QQmlEngine, QString, QUrl};

/// Opens the app with the program's arguments (the program's name first). Returns when its window closes, with the
/// application's exit status (`QGuiApplication::exec`'s: 0, or 1 when a `--screenshot` run found no window to save;
/// 1 when the application couldn't be made), for the process to exit with.
#[must_use]
pub fn run(args: &[OsString]) -> i32 {
    // Qt uses PipeWire for audio when its library is installed, and crashes when PipeWire cannot start (pg_platform::audio).
    // Then the app asks Qt for PulseAudio instead. Qt reads the variable when it makes the audio devices, so it is set here,
    // first, before any thread of the app's own starts. A choice the user made in the environment is kept.
    if std::env::var_os("QT_AUDIO_BACKEND").is_none() && pg_platform::audio::pipewire_context_fails() {
        // SAFETY: no other thread is running yet: this is the first thing `run` does.
        unsafe { std::env::set_var("QT_AUDIO_BACKEND", "pulseaudio") };
    }
    // The crate's and the QML module's C++ initializers. A static library keeps them only where they are called.
    cxx_qt::init_crate!(pg_desktop);
    cxx_qt::init_qml_module!("pg.gallery");
    trace::install();
    pg_trace::pg_stage_main();
    if let Err(e) = command_line::init(args) {
        fatal(&format!("{e:#}"));
    }
    // A screenshot run is a test run: an option it can't use fails it here, on the console, before any window is made.
    if command_line::path("screenshot").is_some() && !command_line::bad_options().is_empty() {
        return refuse_screenshot(command_line::bad_options());
    }
    // `--locale=pseudo` shows every line in the pseudo-locale (the screenshot matrix's pseudo-locale scenario, S-19.3).
    pg_client::text::set_pseudo(command_line::hooks().pseudo);
    // Meanwhile the catalog opens, Qt starts and the QML loads.
    let gpu_thread = gpu_device::make_in_background();
    // Declared before the engine, so dropped after it: Qt Quick renders with the device and keeps no reference.
    let mut gpu: cxx::UniquePtr<gpu_device_bridge::ffi::GpuDevice>;
    // The data folders, from the environment: no folder to keep the data in is an error the window can show, since it
    // is shown before the log exists.
    let mut config = match pg_app::Config::from_env() {
        Ok(config) => config,
        Err(e) => fatal(&e),
    };
    // The log is appended to, so each run's part of it starts with its session's record: the build and where the data is.
    pg_log::init(
        config.layout.log().to_path_buf(),
        &format!("gallery {}, data folder {}", env!("CARGO_PKG_VERSION"), config.layout.home().display()),
    );
    panic_log::install(config.layout.log().to_path_buf());
    // Each option or value the window can't use is logged; it is ignored (the start is the same without it).
    for bad in command_line::bad_options() {
        pg_log::warn("args", &bad.to_string());
    }
    // Sets the thumbnail cache's cap, kept in the cache (`gallery thumbs --cap-gb` too); unset, it stays.
    // A GUI program has no stderr to show a bad value on, so the reason goes to gallery.log.
    config.thumb_cap = std::env::var("PG_THUMB_CAP_GB").ok().and_then(|text| {
        let cap = text.trim().parse::<f64>().ok().and_then(pg_app::reexport::ThumbCap::from_gb);
        if cap.is_none() {
            pg_log::warn(
                "config",
                &format!("PG_THUMB_CAP_GB: {text} isn't a number of GB above zero; the thumbnail cache keeps its cap"),
            );
        }
        cap
    });
    config.yandex = command_line::yandex_source();
    if let Err(e) = app::open(config) {
        fatal(&pg_client::text::render(&pg_client::text::Message::CatalogUnopened { error: format!("{e:#}") }));
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
                unsafe { qml.as_mut().add_image_provider(&QString::from(provider.name()), image_provider) };
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
            .and_then(|thread| thread.join().ok())
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
    let status = app.as_mut().map_or(1, |app| app.exec());
    // The background work stops first (the reader, then the library). Then the engine ends the image providers, and
    // Qt joins their thread pools as they end: the connections those threads hold close with them, before the app
    // closes its own (`app::shutdown`).
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

/// A GUI program has nowhere to print: show the error, then quit.
#[expect(clippy::exit, reason = "the process ends here by design, after the message box")]
fn fatal(message: &str) -> ! {
    alert::error_box(message);
    std::process::exit(1);
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
