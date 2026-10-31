//! How Qt's player decodes videos: on the GPU where it can, else on the CPU.
//!
//! Qt picks a GPU decoder for any codec once it can create a D3D11 device, and falls back to the CPU only when
//! the decoder fails to open. FFmpeg's own AV1 decoder decodes only on the GPU. On a GPU without AV1 decoding it
//! opens fine, then fails every frame, so AV1 videos would stay on their poster. (H.264, HEVC, VP8 and VP9 fall
//! back to the CPU by themselves.) Qt reads `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` once for every codec, so on such
//! a GPU the player decodes everything on the CPU: AV1 through dav1d. A value set by the user is kept.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("pg-ui/cpp/video_decoding.h");
        /// Sets `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` for this GPU (cpp/video_decoding.cpp). Call it before Qt
        /// Multimedia decodes anything.
        #[namespace = "pg"]
        #[cxx_name = "choose_video_decoding"]
        fn choose();
    }
}

pub use ffi::choose;
