// A video's file as the player's device (src/video_source.rs): a QMediaPlayer plays a QFile opened here by the file's
// exact name (Windows: its UTF-16 units, which keep an unpaired surrogate) or by its descriptor (Unix: opened in Rust,
// so the bytes are the file's own). Qt's FFmpeg backend reads the device itself.
#pragma once

#include <QtCore/QPointer>
#include <QtMultimedia/QMediaPlayer>

#include <cstdint>
#include <memory>

#include "rust/cxx.h"

namespace pg {

// One player's source. Only the GUI thread touches it.
class VideoPlayer {
public:
    explicit VideoPlayer(QMediaPlayer* player);
    ~VideoPlayer();
    VideoPlayer(const VideoPlayer&) = delete;
    VideoPlayer& operator=(const VideoPlayer&) = delete;

    // Plays the file named by `units` (UTF-16, as the file system holds it). `ext` is the file's extension, ASCII.
    void play_units(rust::Slice<const std::uint16_t> units, rust::Str ext);
    // Plays the file open at `fd`, which the player then owns and closes. A negative `fd` is a file that could not be
    // opened: the player reports that as its error.
    void play_fd(std::int32_t fd, rust::Str ext);
    // Stops the video: the player's source is cleared, then the device is released.
    void clear();

private:
    // Sets `device` as the player's source. The old device is released only after the player has replaced it: the
    // backend stops reading it first.
    void set_device(std::unique_ptr<QIODevice> device, rust::Str ext);

    QPointer<QMediaPlayer> player_;
    // The device the player plays, kept while it plays.
    std::unique_ptr<QIODevice> device_;
};

std::unique_ptr<VideoPlayer> new_video_player(QMediaPlayer* player);

}  // namespace pg
