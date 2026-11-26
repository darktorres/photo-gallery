// Async image providers: each request becomes a job on a thread pool, which calls Rust's load_image and hands
// the QImage back to the response on the GUI thread.
//
// Newest requests run first (LIFO): while scrolling, the thumbnails now on screen are made before
// the ones that already scrolled past, and a request whose Image went away is dropped before it starts.

#include "providers.h"

#include <QtCore/QMutex>
#include <QtCore/QPointer>
#include <QtCore/QRunnable>
#include <QtCore/QThreadPool>
#include <QtGui/QImage>
#include <QtQuick/QQuickAsyncImageProvider>
#include <QtQuick/QQuickImageResponse>
#include <QtQuick/QQuickTextureFactory>

#include <atomic>
#include <cstdint>
#include <memory>
#include <vector>

#include "pg-desktop/src/bridges/providers_bridge.cxx.h"

namespace pg {
namespace {

class Response;

// What a job and its response share. The response clears `response` when it is destroyed, under the mutex, so a
// job finishing late never touches a deleted object.
struct Link {
    QMutex mutex;
    Response* response = nullptr;
    std::atomic<bool> cancelled{false};
};

// A thumbnail comes decoded (Rust decodes it with QImage::fromData, from the bytes where they are). Rust's pixels
// become the QImage's own: the QImage keeps the buffer and frees it with its last copy. Their formats are ones the
// scene graph uploads as they are (pg_render::display::Rgba, as providers.rs hands it), so neither side copies the image.
QImage to_qimage(Pixels px) {
    if (px.format == PixelFormat::Image) return std::move(px.image);
    if (px.data.empty()) return {};
    QImage::Format format;
    switch (px.format) {
        case PixelFormat::Rgbx8888:
            format = QImage::Format_RGBX8888;
            break;
        case PixelFormat::Rgba8888Premultiplied:
            format = QImage::Format_RGBA8888_Premultiplied;
            break;
        default:
            return {};
    }
    const qsizetype stride = qsizetype(px.width) * 4;
    if (px.width <= 0 || px.height <= 0 || qsizetype(px.data.size()) < stride * px.height) return {};
    auto* owned = new rust::Vec<std::uint8_t>(std::move(px.data));
    auto* bytes = reinterpret_cast<uchar*>(owned->data());
    const auto free = [](void* vec) { delete static_cast<rust::Vec<std::uint8_t>*>(vec); };
    // 32-bit pixels want 4-byte alignment, which Rust's allocator gives (16 on x64); a copy if it ever didn't.
    if (reinterpret_cast<quintptr>(bytes) % 4 != 0) {
        QImage copy = QImage(bytes, px.width, px.height, stride, format).copy();
        free(owned);
        return copy;
    }
    return QImage(bytes, px.width, px.height, stride, format, free, owned);
}

class Response : public QQuickImageResponse {
public:
    explicit Response(std::shared_ptr<Link> link) : m_link(std::move(link)) {
        QMutexLocker lock(&m_link->mutex);
        m_link->response = this;
    }

    ~Response() override {
        QMutexLocker lock(&m_link->mutex);
        m_link->response = nullptr;
    }

    void deliver(QImage image) {
        m_image = std::move(image);
        emit finished();
    }

    QQuickTextureFactory* textureFactory() const override {
        return QQuickTextureFactory::textureFactoryForImage(m_image);
    }

    QString errorString() const override { return m_image.isNull() ? QStringLiteral("image unavailable") : QString(); }

    void cancel() override { m_link->cancelled = true; }

private:
    std::shared_ptr<Link> m_link;
    QImage m_image;
};

class Job : public QRunnable {
public:
    Job(ImageKind kind, QString id, std::shared_ptr<Link> link)
        : m_kind(kind), m_id(std::move(id)), m_link(std::move(link)) {}

    void run() override {
        QImage image;
        if (!m_link->cancelled) {
            const QByteArray id = m_id.toUtf8();
            image = to_qimage(load_image(m_kind, rust::Str(id.constData(), size_t(id.size()))));
        }
        QMutexLocker lock(&m_link->mutex);
        if (Response* r = m_link->response) {
            QMetaObject::invokeMethod(r, [r, image = std::move(image)]() mutable { r->deliver(std::move(image)); },
                                      Qt::QueuedConnection);
        }
    }

private:
    ImageKind m_kind;
    QString m_id;
    std::shared_ptr<Link> m_link;
};

class Provider;

// One is queued on the pool for each request. When it runs, it takes the newest job that has not started yet, so the
// jobs start newest first. The pool orders these runnables all alike, so it needs no priority number, and none can wrap.
class Pump : public QRunnable {
public:
    explicit Pump(Provider* provider) : m_provider(provider) {}
    void run() override;

private:
    Provider* m_provider;
};

class Provider : public QQuickAsyncImageProvider {
public:
    explicit Provider(ImageKind kind) : m_kind(kind) {
        // Thumbnails read originals and decode: enough threads to keep the disk queue full (8 in
        // flight). Full-size images are big; two at a time keeps memory in check. A Yandex picture is a file the search
        // fetches on demand, so four at a time.
        m_pool.setMaxThreadCount(kind == ImageKind::Thumb ? 8 : kind == ImageKind::Yandex ? 4 : 2);
    }

    ~Provider() override {
        m_pool.clear();
        // The idle threads end now, not after their expiry: each one's thread-local connections close as it ends.
        m_pool.setExpiryTimeout(0);
        m_pool.waitForDone();
    }

    QQuickImageResponse* requestImageResponse(const QString& id, const QSize&) override {
        auto link = std::make_shared<Link>();
        auto* response = new Response(link);
        {
            QMutexLocker lock(&m_mutex);
            m_waiting.push_back(std::make_unique<Job>(m_kind, id, link));
        }
        m_pool.start(new Pump(this));
        return response;
    }

    // The newest job that has not started, or none when they have all started.
    std::unique_ptr<Job> take_newest() {
        QMutexLocker lock(&m_mutex);
        if (m_waiting.empty()) return nullptr;
        std::unique_ptr<Job> job = std::move(m_waiting.back());
        m_waiting.pop_back();
        return job;
    }

private:
    ImageKind m_kind;
    QThreadPool m_pool;
    QMutex m_mutex;
    // The requests not yet started, oldest first.
    std::vector<std::unique_ptr<Job>> m_waiting;
};

void Pump::run() {
    if (std::unique_ptr<Job> job = m_provider->take_newest()) job->run();
}

}  // namespace

QQmlImageProviderBase* new_image_provider(ImageKind kind) { return new Provider(kind); }

}  // namespace pg
