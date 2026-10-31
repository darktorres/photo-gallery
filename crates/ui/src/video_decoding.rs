//! How Qt's player decodes videos: on the GPU where it can, else on the CPU.
//!
//! Qt picks a GPU decoder for any codec once it can create a D3D11 device, and falls back to the CPU only when
//! the decoder fails to open. FFmpeg's own AV1 decoder decodes only on the GPU. On a GPU without AV1 decoding it
//! opens fine, then fails every frame, so AV1 videos would stay on their poster. (H.264, HEVC, VP8 and VP9 fall
//! back to the CPU by themselves.) Qt reads `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` once for every codec, so on such
//! a GPU the player decodes everything on the CPU: AV1 through dav1d; on one with AV1 decoding, through D3D11.
//!
//! Both of Qt's lists are always set, the encoders' to none: without a list Qt probes every kind of device FFmpeg
//! knows (CUDA, QSV, Vulkan…), 1.4 s on the GUI thread when the first player is made (FINDINGS "Startup"). A value
//! set by the user is kept.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-ui/cpp/video_decoding.h");
        /// Sets `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` for this GPU, and `QT_FFMPEG_ENCODING_HW_DEVICE_TYPES` to none
        /// (cpp/video_decoding.cpp). Call it before Qt Multimedia is first used; any thread.
        #[namespace = "pg"]
        #[cxx_name = "choose_video_decoding"]
        fn choose();
    }
}

/// [`ffi::choose`] on its own thread (it makes a D3D11 device, ~0.3 s): join the handle before the QML loads.
pub fn choose_in_background() -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("video decoding".into())
        .spawn(ffi::choose)
        .expect("spawn the video decoding probe")
}
