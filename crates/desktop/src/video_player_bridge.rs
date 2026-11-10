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
        /// Plays the file open at `handle` (`pg_platform::file::player_handle`: a `HANDLE` on Windows, a descriptor on
        /// Unix), which the player then owns and closes. A negative `handle` is a file that could not be opened: the
        /// player reports that as its error. `ext` is the file's extension, ASCII.
        #[namespace = "pg"]
        fn play_file(self: Pin<&mut VideoPlayer>, handle: i64, ext: &str);
        /// Clears the player's source, then releases the device.
        #[namespace = "pg"]
        fn clear(self: Pin<&mut VideoPlayer>);
    }
}
