//! The cxx-qt bridge of [`crate::deadline`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-desktop/cpp/deadline.h");
        type QObject = cxx_qt::QObject;
        #[namespace = "pg"]
        unsafe fn deadline_arm(owner: *mut QObject, ms: i64);
        #[namespace = "pg"]
        unsafe fn deadline_disarm(owner: *mut QObject);
        #[namespace = "pg"]
        fn deadline_now_ms() -> i64;
    }
}
