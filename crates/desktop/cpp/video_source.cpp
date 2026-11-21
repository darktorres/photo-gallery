#include "video_source.h"

#include <QtCore/QFile>
#include <QtCore/QString>
#include <QtCore/QUrl>

#ifdef Q_OS_WIN
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

VideoPlayer::VideoPlayer(QMediaPlayer* player) : m_player(player) {}

VideoPlayer::~VideoPlayer() {
    // The player stops reading the device before the device is released (it is a member, released after this).
    if (m_player) {
        m_player->setSource(QUrl());
    }
}

void VideoPlayer::play_file(std::int64_t handle, rust::Str ext) {
    auto file = std::make_unique<QFile>();
    if (handle >= 0) {
#ifdef Q_OS_WIN
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
#ifdef Q_OS_WIN
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
    if (!m_player || !m_device) {
        return;
    }
    // Declared before the call, so it is released after the player has stopped reading it.
    std::unique_ptr<QIODevice> old = std::move(m_device);
    m_player->setSource(QUrl());
}

void VideoPlayer::set_device(std::unique_ptr<QIODevice> device, rust::Str ext) {
    if (!m_player) {
        m_device = std::move(device);
        return;
    }
    std::unique_ptr<QIODevice> old = std::move(m_device);
    QIODevice* next = device.get();
    m_device = std::move(device);
    m_player->setSourceDevice(next, hint_url(ext));
    // `old` is released here, after the player has replaced it.
}

std::unique_ptr<VideoPlayer> new_video_player(QMediaPlayer* player) {
    return std::make_unique<VideoPlayer>(player);
}

}  // namespace pg
