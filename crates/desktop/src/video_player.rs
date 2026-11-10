//! The video player's device (cpp/video_source.h): a `QMediaPlayer` plays a file handed over by its exact name. Windows
//! names it by its UTF-16 units; Unix opens it here and hands over the descriptor, so a name with an unpaired surrogate or
//! a raw byte plays as well as any other (Qt's file URL can't carry those names). A bridge with no QObject, below the
//! `VideoSource` bridge and its state, which both name its types. The one place the player's per-platform calls are
//! chosen: their declarations differ by platform, so the choice is FFI glue (MODULARITY_PLAN ground rule 6).

use core::pin::Pin;
use std::ffi::OsStr;
use std::path::Path;

use crate::video_player_bridge::ffi;

/// Plays the file at `path`, in the form the system holds its name in (`pg_platform::file::play_target`): its units on
/// Windows, its open descriptor on Unix. The player's declarations (`video_player_bridge`) are per platform, to take
/// that form.
pub(crate) fn play_path(player: Pin<&mut ffi::VideoPlayer>, path: &Path) {
    let ext = ext_of(path);
    match pg_platform::file::play_target(path) {
        #[cfg(windows)]
        pg_platform::file::PlayTarget::Units(units) => player.play_units(&units, ext),
        #[cfg(unix)]
        pg_platform::file::PlayTarget::Descriptor(fd) => player.play_fd(fd, ext),
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
