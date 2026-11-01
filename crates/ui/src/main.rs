//! `gallery`: the photo gallery (PLAN §7). Without arguments it opens the app, which imports the library in the
//! background; with a command (`gallery scan`, `gallery import`, … see `pg_ingest::cli`) it runs that pass on the
//! command line instead. Qt Quick over CXX-Qt; everything below the QML lives in `pg-core` and `pg-ingest`.

// A GUI program: no console window when started from Explorer. The command line attaches to its caller's console.
#![windows_subsystem = "windows"]

mod animation;
mod app_icon;
mod command_line;
mod filters;
mod folders;
mod gallery;
mod gpu_device;
mod library;
#[cfg(debug_assertions)]
mod live_reload;
mod loader;
mod panic_log;
mod providers;
mod screenshot;
mod settings;
mod state;
mod trace;
mod view;
mod yandex;

use std::ffi::OsString;
use std::path::PathBuf;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQmlEngine, QString, QUrl};
use pg_ingest::cli;

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    if cli::wants_cli(&args[1..]) {
        attach_console();
        if let Err(e) = cli::main(args) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    run_app();
}

fn run_app() {
    trace::pg_stage_main();
    // Meanwhile the catalog opens and Qt starts.
    let gpu_thread = gpu_device::make_in_background();
    // Declared before the engine, so dropped after it: Qt Quick renders with the device and keeps no reference.
    let mut gpu: cxx::UniquePtr<gpu_device::ffi::GpuDevice>;
    let catalog_dir =
        std::env::var_os("PG_CATALOG_DIR").map_or_else(|| PathBuf::from(cli::DEFAULT_CATALOG_DIR), PathBuf::from);
    panic_log::install(catalog_dir.parent().unwrap_or(&catalog_dir).join("gallery.log"));
    // Sets the thumbnail cache's cap, kept in the cache (`gallery thumbs --cap-gb` too); unset, it stays.
    let cap = std::env::var("PG_THUMB_CAP_GB").ok().and_then(|s| match s.parse::<f64>() {
        Ok(gb) => Some((gb * 1e9) as i64),
        Err(_) => {
            eprintln!("PG_THUMB_CAP_GB={s} isn't a number of GB: the thumbnail cache keeps its cap");
            None
        }
    });
    if let Err(e) = state::init(catalog_dir, cap) {
        fatal(&format!("Can't open the catalog: {e:#}"));
    }
    trace::pg_stage_init_done();

    // The controls style is Fusion, imported by name in the QML (`QtQuick.Controls.Fusion`): it follows the
    // window's dark palette. The default on Windows, the native style, draws light system buttons regardless.
    let mut app = QGuiApplication::new();
    app_icon::ffi::set_app_icon();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        {
            let mut qml: core::pin::Pin<&mut QQmlEngine> = engine.as_mut().upcast_pin();
            for (name, kind) in [
                ("thumb", providers::KIND_THUMB),
                ("thumbhash", providers::KIND_THUMBHASH),
                ("full", providers::KIND_FULL),
            ] {
                // SAFETY: a new provider, which the engine takes ownership of.
                unsafe {
                    qml.as_mut().add_image_provider(&QString::from(name), providers::ffi::new_image_provider(kind))
                };
            }
        }
        gpu = gpu_thread.join().expect("the gpu device thread");
        // Qt Multimedia reads it when the QML makes the first player.
        gpu_device::ffi::choose_video_decoding(&gpu);
        let main_qml = QUrl::from("qrc:/qt/qml/pg/gallery/qml/Main.qml");
        #[cfg(debug_assertions)]
        // SAFETY: `gpu` is dropped after the engine.
        unsafe {
            live_reload::ffi::enable_live_reload(
                engine.as_mut(),
                &main_qml,
                &QString::from(concat!(env!("CARGO_MANIFEST_DIR"), "/qml")),
                gpu.pin_mut(),
            )
        };
        trace::pg_stage_qml_load();
        engine.as_mut().load(&main_qml);
        trace::pg_stage_qml_loaded();
        // SAFETY: as above.
        unsafe { gpu_device::ffi::show_windows(engine.as_mut(), gpu.pin_mut()) };
        screenshot::schedule(engine);
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    loader::stop();
    library::stop();
    state::shutdown();
}

/// Make stdout and stderr reach the console the command was typed in. A GUI-subsystem program starts without
/// them unless they were redirected (a pipe or a file, which then stay as they are).
fn attach_console() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    let valid = |h: HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
    // SAFETY: GetStdHandle and AttachConsole take no pointers.
    let missing: Vec<STD_HANDLE> =
        [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().filter(|&s| !valid(unsafe { GetStdHandle(s) })).collect();
    // SAFETY: as above.
    if missing.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; the null pointers are optional arguments.
    let out = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if valid(out) {
        for s in missing {
            // SAFETY: `out` is a valid console handle, kept open for the rest of the process.
            unsafe { SetStdHandle(s, out) };
        }
    }
}

/// A GUI program has nowhere to print: show the error, then quit.
fn fatal(message: &str) -> ! {
    message_box(message);
    std::process::exit(1);
}

/// An error message box, from any thread (it runs its own message loop).
fn message_box(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("Photo Gallery"));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
}
