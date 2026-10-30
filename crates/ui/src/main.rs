//! `gallery`: the desktop UI (PLAN §7, M2). Qt Quick over CXX-Qt; everything below the QML lives in `pg-core`.

mod folders;
mod gallery;
mod providers;
mod state;

use std::path::PathBuf;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQmlEngine, QString, QUrl};

fn main() {
    let catalog_dir =
        std::env::var_os("PG_CATALOG_DIR").map_or_else(|| PathBuf::from(r"D:\PhotoGallery\catalog"), PathBuf::from);
    let cap_gb: f64 = std::env::var("PG_THUMB_CAP_GB").ok().and_then(|s| s.parse().ok()).unwrap_or(50.0);
    if let Err(e) = state::init(catalog_dir, (cap_gb * 1e9) as i64) {
        eprintln!("can't open the catalog: {e:#}");
        std::process::exit(1);
    }

    // SAFETY: still single-threaded; Qt reads these when the application starts.
    unsafe {
        std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Fusion");
    }
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
