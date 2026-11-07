#include "video_source.h"

#include <QtCore/QFile>
#include <QtCore/QString>
#include <QtCore/QUrl>

#if !defined(_WIN32)
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

void VideoPlayer::play_units(rust::Slice<const std::uint16_t> units, rust::Str ext) {
    // The units as they are: a QString holds an unpaired surrogate, and QFile hands the name to CreateFileW unchanged.
    const QString name(reinterpret_cast<const QChar*>(units.data()), static_cast<qsizetype>(units.size()));
    set_device(std::make_unique<QFile>(name), ext);
}

void VideoPlayer::play_fd(std::int32_t fd, rust::Str ext) {
    auto file = std::make_unique<QFile>();
    if (fd >= 0 && !file->open(fd, QIODevice::ReadOnly, QFileDevice::AutoCloseHandle)) {
        // Qt takes the descriptor only when it opens: a refused one is closed here, or it would stay open.
#if !defined(_WIN32)
        ::close(fd);
#endif
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
