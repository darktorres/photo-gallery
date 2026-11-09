//! The desktop app (PLAN §7): the window, Qt Quick over CXX-Qt. `gallery` (crates/gallery) opens it with `run` when it's
//! given no command; everything below the QML lives in `pg-client` and `pg-library`.

mod animation;
mod app;
mod app_icon;
mod command_line;
mod controller;
mod deadline;
mod facet_choices;
mod filters;
mod folders;
mod gallery;
mod gpu_device;
mod gui_app;
mod keymap;
mod library;
#[cfg(debug_assertions)]
mod live_reload;
mod panic_log;
mod picker;
mod providers;
mod screenshot;
mod settings;
mod shell;
mod trace;
mod video_source;
mod viewport;
mod words;

mod yandex;
mod yandex_log;

use std::ffi::OsString;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QQmlApplicationEngine, QQmlEngine, QString, QUrl};

/// Opens the app with the program's arguments (the program's name first). Returns when its window closes, with the
/// application's exit status (`QGuiApplication::exec`'s: 0, or 1 when a `--screenshot` run found no window to save;
/// 1 when the application couldn't be made), for the process to exit with.
#[must_use]
pub fn run(args: &[OsString]) -> i32 {
    // The crate's and the QML module's C++ initializers. A static library keeps them only where they are called.
    cxx_qt::init_crate!(pg_desktop);
    cxx_qt::init_qml_module!("pg.gallery");
    trace::install();
    pg_trace::pg_stage_main();
    if let Err(e) = command_line::init(args) {
        fatal(&format!("{e:#}"));
    }
    // `--locale=pseudo` shows every line in the pseudo-locale (the screenshot matrix's pseudo-locale scenario, S-19.3).
    pg_client::text::set_pseudo(command_line::value("locale") == Some("pseudo"));
    // Meanwhile the catalog opens, Qt starts and the QML loads.
    let gpu_thread = gpu_device::make_in_background();
    // Declared before the engine, so dropped after it: Qt Quick renders with the device and keeps no reference.
    let mut gpu: cxx::UniquePtr<gpu_device::ffi::GpuDevice>;
    // The catalog folder and the preferences file, from the environment.
    let mut config = pg_app::Config::from_env();
    panic_log::install(config.catalog_dir.parent().unwrap_or(&config.catalog_dir).join("gallery.log"));
    // Sets the thumbnail cache's cap, kept in the cache (`gallery thumbs --cap-gb` too); unset, it stays.
    // A GUI program has no stderr to show a bad value on, so the reason goes to gallery.log.
    config.thumb_cap_bytes =
        std::env::var("PG_THUMB_CAP_GB").ok().and_then(|s| match pg_app::reexport::parse_thumb_cap_gb(&s) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                panic_log::note(&format!("PG_THUMB_CAP_GB: {e}; the thumbnail cache keeps its cap"));
                None
            }
        });
    if let Err(e) = app::open(config) {
        fatal(&pg_client::text::catalog_unopened(&format!("{e:#}")));
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
            for (name, kind) in [
                ("thumb", providers::ffi::ImageKind::Thumb),
                ("thumbhash", providers::ffi::ImageKind::Thumbhash),
                ("full", providers::ffi::ImageKind::Full),
                ("yandex", providers::ffi::ImageKind::Yandex),
            ] {
                // SAFETY: a new provider, which the engine takes ownership of.
                unsafe {
                    qml.as_mut().add_image_provider(&QString::from(name), providers::ffi::new_image_provider(kind))
                };
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
        gpu = gpu_thread.ok().and_then(|thread| thread.join().ok()).unwrap_or_else(gpu_device::ffi::make_gpu_device);
        gpu_device::ffi::choose_video_decoding(&gpu);
        #[cfg(debug_assertions)]
        // SAFETY: `gpu` is dropped after the engine.
        unsafe {
            live_reload::ffi::reload_on_change(engine.as_mut(), &main_qml, &qml_dir, gpu.pin_mut())
        };
        // SAFETY: as above.
        unsafe { gpu_device::ffi::show_windows(engine.as_mut(), gpu.pin_mut()) };
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

/// A GUI program has nowhere to print: show the error, then quit.
#[expect(clippy::exit, reason = "the process ends here by design, after the message box")]
fn fatal(message: &str) -> ! {
    message_box(message);
    std::process::exit(1);
}

/// An error message box, from any thread (it runs its own message loop).
fn message_box(message: &str) {
    pg_platform::message::error_box("Photo Gallery", message);
}
