//! Debug builds read the QML from `crates/desktop/qml` instead of the copy compiled into the program, and reload the
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
        include!("pg-desktop/cpp/gpu_device.h");
        #[namespace = "pg"]
        type GpuDevice = crate::gpu_device::ffi::GpuDevice;

        include!("pg-desktop/cpp/live_reload.h");
        /// Fetch the QML files in `main`'s qrc: folder from `source_dir` instead. Call before the first load.
        #[namespace = "pg"]
        fn read_qml_from_source(engine: Pin<&mut QQmlApplicationEngine>, main: &QUrl, source_dir: &QString);

        /// Load `main` again whenever a file in `source_dir` changes; each new window is shown with
        /// `gpu_device::ffi::show_windows`.
        ///
        /// # Safety
        ///
        /// `gpu` outlives the engine: the reloads use it.
        #[namespace = "pg"]
        unsafe fn reload_on_change(
            engine: Pin<&mut QQmlApplicationEngine>,
            main: &QUrl,
            source_dir: &QString,
            gpu: Pin<&mut GpuDevice>,
        );
    }
}
