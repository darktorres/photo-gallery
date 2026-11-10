//! The cxx-qt bridge of [`crate::gpu_device`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("pg-desktop/cpp/gpu_device.h");
        /// The device and what was learned on it.
        #[namespace = "pg"]
        type GpuDevice;

        /// Make the device on the default adapter and look for the AV1 decoder on it (~0.3 s). Any thread.
        #[namespace = "pg"]
        fn make_gpu_device() -> UniquePtr<GpuDevice>;

        /// Sets `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` for this GPU, and `QT_FFMPEG_ENCODING_HW_DEVICE_TYPES` to none.
        /// Call it on the main thread, before Qt Multimedia is first used.
        #[namespace = "pg"]
        fn choose_video_decoding(gpu: &GpuDevice);

        /// Show the engine's windows, rendering with the device when Qt Quick would make the same one itself; else
        /// the device is let go. After every load of the QML.
        ///
        /// # Safety
        ///
        /// `gpu` outlives the windows: Qt keeps no reference to the device.
        #[namespace = "pg"]
        unsafe fn show_windows(engine: Pin<&mut QQmlApplicationEngine>, gpu: Pin<&mut GpuDevice>);
    }
}
