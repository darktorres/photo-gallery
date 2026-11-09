//! The video player's source (`VideoSource`): the file a video plays from, handed to Qt's player as a device
//! (cpp/video_source.cpp). The file is the catalog's exact path: Windows names it by its UTF-16 units, Unix opens it
//! here and hands over the descriptor, so a name with an unpaired surrogate or a raw byte plays as well as any other.
//! Qt's file URL can't carry those names. Only the GUI thread touches it.

#[cxx_qt::bridge]
pub mod qobject {
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

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type VideoSource = super::VideoSourceRust;
    }

    unsafe extern "RustQt" {
        /// The player the video plays on (QML's `MediaPlayer`). Called once, when the source is made.
        ///
        /// # Safety
        /// `player` is a live `QMediaPlayer` or null (see `new_video_player`).
        #[qinvokable]
        unsafe fn attach(self: Pin<&mut VideoSource>, player: *mut QMediaPlayer);

        /// Plays the video of the catalog's file `file_id` from the start. A file the catalog doesn't have clears the
        /// player instead.
        #[qinvokable]
        fn play(self: Pin<&mut VideoSource>, file_id: i64);

        /// Stops the video: the player's source is cleared.
        #[qinvokable]
        fn stop(self: Pin<&mut VideoSource>);
    }
}

use core::pin::Pin;
use std::ffi::OsStr;
use std::path::Path;

use cxx::UniquePtr;
use cxx_qt::CxxQtType;
use pg_model::ids::FileId;

use self::qobject::VideoPlayer;

pub struct VideoSourceRust {
    /// Null until `attach`.
    player: UniquePtr<VideoPlayer>,
}

impl Default for VideoSourceRust {
    fn default() -> Self {
        Self { player: UniquePtr::null() }
    }
}

/// The player, when one is attached.
fn player_of(rust: &mut VideoSourceRust) -> Option<Pin<&mut VideoPlayer>> {
    if rust.player.is_null() { None } else { Some(rust.player.pin_mut()) }
}

impl qobject::VideoSource {
    /// # Safety
    /// As `qobject::new_video_player`: `player` is a live `QMediaPlayer` or null.
    unsafe fn attach(mut self: Pin<&mut Self>, player: *mut qobject::QMediaPlayer) {
        // SAFETY: the caller's contract; the C++ side keeps only a weak reference to the player.
        let player = unsafe { qobject::new_video_player(player) };
        self.as_mut().rust_mut().get_mut().player = player;
    }

    fn play(mut self: Pin<&mut Self>, file_id: i64) {
        let path = crate::app::get().file_path(FileId::new(file_id));
        let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) else { return };
        match path {
            Ok(path) => play_path(player, &path),
            Err(err) => {
                crate::log::note(&format!("video {file_id}: {err:#}"));
                player.clear();
            }
        }
    }

    fn stop(mut self: Pin<&mut Self>) {
        if let Some(player) = player_of(self.as_mut().rust_mut().get_mut()) {
            player.clear();
        }
    }
}

/// Plays the file at `path`: its name's units on Windows, its open descriptor on Unix.
fn play_path(player: Pin<&mut VideoPlayer>, path: &Path) {
    let ext = ext_of(path);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        player.play_units(&units, ext);
    }
    #[cfg(unix)]
    {
        player.play_fd(fd_of(path), ext);
    }
}

/// The file open for reading, as a descriptor the player takes over (`-1` when it can't be opened: the player then
/// reports the error).
#[cfg(unix)]
fn fd_of(path: &Path) -> i32 {
    use std::os::unix::io::IntoRawFd;
    match std::fs::File::open(path) {
        Ok(file) => file.into_raw_fd(),
        Err(_) => -1,
    }
}

/// The file's extension, when it is ASCII: the player's format probe reads it from the name it is given, so the name
/// is the extension alone (the file's own name never reaches the player). Empty otherwise.
fn ext_of(path: &Path) -> &str {
    path.extension().and_then(OsStr::to_str).filter(|e| e.bytes().all(|b| b.is_ascii_alphanumeric())).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_is_the_extension_when_it_is_ascii() {
        assert_eq!(ext_of(Path::new("D:/clips/a.MOV")), "MOV");
        assert_eq!(ext_of(Path::new("D:/clips/no-extension")), "");
        assert_eq!(ext_of(Path::new("D:/clips/a.mö")), "");
    }
}
