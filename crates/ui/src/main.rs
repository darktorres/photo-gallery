//! `gallery`: the photo gallery (PLAN §7). Without arguments it opens the app, which imports the library in the
//! background; with a command (`gallery scan`, `gallery import`, … see `pg_ingest::cli`) it runs that pass on the
//! command line instead. Qt Quick over CXX-Qt; everything below the QML lives in `pg-core` and `pg-ingest`.

// A GUI program: no console window when started from Explorer. The command line attaches to its caller's console.
#![windows_subsystem = "windows"]

mod folders;
mod gallery;
mod library;
mod providers;
mod state;

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
    let catalog_dir =
        std::env::var_os("PG_CATALOG_DIR").map_or_else(|| PathBuf::from(cli::DEFAULT_CATALOG_DIR), PathBuf::from);
    let cap_gb: f64 = std::env::var("PG_THUMB_CAP_GB").ok().and_then(|s| s.parse().ok()).unwrap_or(50.0);
    if let Err(e) = state::init(catalog_dir, (cap_gb * 1e9) as i64) {
        fatal(&format!("Can't open the catalog: {e:#}"));
    }

    // The controls style is Fusion, imported by name in the QML (`QtQuick.Controls.Fusion`): it follows the
    // window's dark palette. The default on Windows, the native style, draws light system buttons regardless.
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        {
            let mut qml: core::pin::Pin<&mut QQmlEngine> = engine.as_mut().upcast_pin();
            for (name, kind) in [
                ("thumb", providers::KIND_THUMB),
                ("thumbhash", providers::KIND_THUMBHASH),
                ("full", providers::KIND_FULL),
            ] {
                // SAFETY: the engine takes ownership of the provider.
                unsafe {
                    qml.as_mut().add_image_provider(&QString::from(name), providers::ffi::new_image_provider(kind))
                };
            }
        }
        engine.load(&QUrl::from("qrc:/qt/qml/pg/gallery/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
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
    let missing: Vec<STD_HANDLE> =
        [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().filter(|&s| !valid(unsafe { GetStdHandle(s) })).collect();
    if missing.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
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
            unsafe { SetStdHandle(s, out) };
        }
    }
}

/// A GUI program has nowhere to print: show the error, then quit.
fn fatal(message: &str) -> ! {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("Photo Gallery"));
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
    std::process::exit(1);
}
