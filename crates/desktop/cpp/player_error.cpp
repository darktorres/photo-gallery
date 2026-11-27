// The kind of a media player's error (src/bridges/animation_bridge.rs), as the window's words need it.

#include "player_error.h"

namespace pg {

std::int32_t player_error_kind(QMediaPlayer* player) {
    if (player == nullptr) {
        return 0;
    }
    switch (player->error()) {
        case QMediaPlayer::NoError:
            return 0;
        case QMediaPlayer::ResourceError:
            return 1;
        case QMediaPlayer::FormatError:
            return 2;
        default:
            return 3;
    }
}

}  // namespace pg
