//! The cxx-qt bridge of [`crate::video_player`]: the declarations alone, so that the allow the macro's output needs is
//! on this module and on nothing written by hand (`lib.rs`).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!(<QtMultimedia/QMediaPlayer>);
        type QMediaPlayer;

        include!("pg-desktop/cpp/video_source.h");
        #[namespace = "pg"]
        type VideoPlayer;
        /// The source of `player` (a live `QMediaPlayer`, or null: the source keeps a weak reference to it).
        #[namespace = "pg"]
        unsafe fn new_video_player(player: *mut QMediaPlayer) -> UniquePtr<VideoPlayer>;
        /// Plays the file named by `units` (Windows: the file's UTF-16 units). `ext` is its extension, ASCII.
        #[cfg(windows)]
        #[namespace = "pg"]
        fn play_units(self: Pin<&mut VideoPlayer>, units: &[u16], ext: &str);
        /// Plays the file open at `fd` (Unix; a negative `fd` is a file that could not be opened).
        #[cfg(unix)]
        #[namespace = "pg"]
        fn play_fd(self: Pin<&mut VideoPlayer>, fd: i32, ext: &str);
        /// Clears the player's source, then releases the device.
        #[namespace = "pg"]
        fn clear(self: Pin<&mut VideoPlayer>);
    }
}
