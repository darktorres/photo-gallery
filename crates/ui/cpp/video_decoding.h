// How Qt's player decodes videos (src/video_decoding.rs).
#pragma once

namespace pg {

// Turns GPU decoding off when the GPU can't decode AV1, unless QT_FFMPEG_DECODING_HW_DEVICE_TYPES is already set.
void choose_video_decoding();

}  // namespace pg
