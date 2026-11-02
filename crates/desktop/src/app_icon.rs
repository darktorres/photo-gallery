//! The window icon (cpp/app_icon.cpp): the title bar and taskbar of each window. The exe's icon, embedded by build.rs,
//! is only what Explorer shows.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-desktop/cpp/app_icon.h");

        /// Set the application's window icon. Call after the QGuiApplication is created.
        #[namespace = "pg"]
        fn set_app_icon();
    }
}
