//! Debug builds read the QML from `crates/ui/qml` instead of the copy compiled into the program, and reload the
//! window whenever a file there changes (cpp/live_reload.cpp): QML edits show in the running app without a
//! rebuild. Release builds leave this out (build.rs).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;

        include!("pg-ui/cpp/live_reload.h");
        /// Fetch the QML files in `main`'s qrc: folder from `source_dir` instead, and load `main` again whenever
        /// one of them changes. Call before the first load.
        #[namespace = "pg"]
        fn enable_live_reload(engine: Pin<&mut QQmlApplicationEngine>, main: &QUrl, source_dir: &QString);
    }
}
