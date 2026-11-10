//! The video player's device (cpp/video_source.h): a `QMediaPlayer` plays a file handed over open, by its raw handle
//! (`pg_platform::file::player_handle`). Qt is never given the file's name: on Windows its file engine normalizes every
//! name, the verbatim one too, which drops a folder's trailing dot, and a name with an unpaired surrogate or a raw byte
//! has no URL form. A bridge with no QObject, below the `VideoSource` bridge and its state, which both name its types.

use core::pin::Pin;
use std::ffi::OsStr;
use std::path::Path;

use crate::video_player_bridge::ffi;

/// Plays the file at `path` (the catalog's `disk_path` of it): opened here, and handed to the player open. A file that
/// can't be opened is logged, and the player reports it as its error.
pub(crate) fn play_path(player: Pin<&mut ffi::VideoPlayer>, path: &Path) {
    let handle = pg_platform::file::player_handle(path).unwrap_or_else(|e| {
        pg_log::warn("video", &format!("can't open {}: {e}", path.display()));
        -1
    });
    player.play_file(handle, ext_of(path));
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
