// A media player's error, as the window words it (src/bridges/animation_bridge.rs): QML cannot compare
// QMediaPlayer::Error with a number, so C++ maps it.
#pragma once

#include <QtMultimedia/QMediaPlayer>

#include <cstdint>

namespace pg {

// The kind of `player`'s error: 0 none, 1 the source could not be opened (ResourceError), 2 its format is not supported
// (FormatError), 3 any other failure. A null player has none. Call on the GUI thread.
std::int32_t player_error_kind(QMediaPlayer* player);

}  // namespace pg
