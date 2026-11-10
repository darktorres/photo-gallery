#include "video_source.h"

#include <QtCore/QFile>
#include <QtCore/QString>
#include <QtCore/QUrl>

#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <fcntl.h>
#include <io.h>
#else
#include <unistd.h>
#endif

namespace pg {
namespace {

// The URL the backend names the device by. The backend probes the format by its extension and never opens the URL,
// so the name is ASCII and the file's own name stays out of it.
QUrl hint_url(rust::Str ext) {
    QString name = QStringLiteral("file:///pg-video");
    if (!ext.empty()) {
        name += QStringLiteral(".") + QString::fromUtf8(ext.data(), static_cast<qsizetype>(ext.size()));
    }
    return QUrl(name);
}

}  // namespace

VideoPlayer::VideoPlayer(QMediaPlayer* player) : player_(player) {}

VideoPlayer::~VideoPlayer() {
    // The player stops reading the device before the device is released (it is a member, released after this).
    if (player_) {
        player_->setSource(QUrl());
    }
}

void VideoPlayer::play_file(std::int64_t handle, rust::Str ext) {
    auto file = std::make_unique<QFile>();
    if (handle >= 0) {
#if defined(_WIN32)
        // The C runtime's descriptor over the handle: the workspace links the runtime dynamically (no crt-static), so
        // Rust, this code and Qt share one table, and QFile reads the handle back from it (_get_osfhandle).
        const int fd = _open_osfhandle(static_cast<intptr_t>(handle), _O_RDONLY | _O_BINARY);
        if (fd < 0) {
            // No descriptor took the handle over: it is closed here, or it would stay open.
            CloseHandle(reinterpret_cast<HANDLE>(static_cast<intptr_t>(handle)));
        }
#else
        const int fd = static_cast<int>(handle);
#endif
        if (fd >= 0 && !file->open(fd, QIODevice::ReadOnly, QFileDevice::AutoCloseHandle)) {
            // Qt takes the descriptor only when it opens: a refused one is closed here, or it would stay open.
#if defined(_WIN32)
            _close(fd);
#else
            ::close(fd);
#endif
        }
    }
    set_device(std::move(file), ext);
}

void VideoPlayer::clear() {
    // Without a device there is no source to clear, and no source change to report.
    if (!player_ || !device_) {
        return;
    }
    // Declared before the call, so it is released after the player has stopped reading it.
    std::unique_ptr<QIODevice> old = std::move(device_);
    player_->setSource(QUrl());
}

void VideoPlayer::set_device(std::unique_ptr<QIODevice> device, rust::Str ext) {
    if (!player_) {
        device_ = std::move(device);
        return;
    }
    std::unique_ptr<QIODevice> old = std::move(device_);
    QIODevice* next = device.get();
    device_ = std::move(device);
    player_->setSourceDevice(next, hint_url(ext));
    // `old` is released here, after the player has replaced it.
}

std::unique_ptr<VideoPlayer> new_video_player(QMediaPlayer* player) {
    return std::make_unique<VideoPlayer>(player);
}

}  // namespace pg
