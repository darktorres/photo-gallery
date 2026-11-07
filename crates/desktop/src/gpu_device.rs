//! The app's one D3D11 device (cpp/gpu_device.cpp): Qt Quick renders with it, and Qt's player decodes on the GPU
//! only if the GPU's decoder for AV1 is found on it. Making a device costs ~0.9 G instructions here, most of it the
//! driver's loader; before, the decoder lookup and Qt Quick each made one (PROFILING "The start, to ready").
//!
//! **Rendering.** The window is declared `visible: false` and shown by [`ffi::show_windows`], which first hands Qt
//! the device, unless Qt would make a different one: another backend (`QSG_RHI_BACKEND`), another adapter
//! (`QT_D3D_ADAPTER_INDEX`), the debug layer, a software device, or `PG_SHARED_DEVICE=0`. Once Qt's renderer is up
//! the hand-over is forgotten, so after a lost device (a driver reset) Qt makes its own instead of reusing the dead
//! one. `QSG_INFO=1` prints "Using imported device"; `QT_LOGGING_RULES="pg.gpu.info=true"` says what was decided.
//!
//! **Decoding.** Qt picks a GPU decoder for any codec once it can create a D3D11 device, and falls back to the CPU
//! only when the decoder fails to open. FFmpeg's own AV1 decoder decodes only on the GPU. On a GPU without AV1
//! decoding it opens fine, then fails every frame, so AV1 videos would stay on their poster. (H.264, HEVC, VP8 and
//! VP9 fall back to the CPU by themselves.) Qt reads `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` once for every codec, so
//! on such a GPU the player decodes everything on the CPU: AV1 through dav1d; on one with AV1 decoding, through
//! D3D11. `PG_FORCE_AV1=1` takes the GPU branch on any GPU, for testing.
//!
//! Both of Qt's lists are always set, the encoders' to none: without a list Qt probes every kind of device FFmpeg
//! knows (CUDA, QSV, Vulkan…), 1.4 s on the GUI thread when the first player is made (FINDINGS "Startup"). A value
//! set by the user is kept.

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

// SAFETY: the device is free-threaded; its immediate context is used by one thread at a time (made on the
// "gpu device" thread, then handed to the GUI thread, which hands it to the render thread).
unsafe impl Send for ffi::GpuDevice {}

/// [`ffi::make_gpu_device`] on its own thread, while the catalog opens and the QML loads: join the handle before
/// the window is shown and before Qt Multimedia is first used (the viewer's media Loader, Viewer.qml). An error when
/// the thread can't be made: the caller then makes the device itself.
pub fn make_in_background() -> std::io::Result<std::thread::JoinHandle<cxx::UniquePtr<ffi::GpuDevice>>> {
    std::thread::Builder::new().name("gpu device".into()).spawn(ffi::make_gpu_device)
}
