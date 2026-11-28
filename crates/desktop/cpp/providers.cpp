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

// What a provider's pump and the provider share: the requests not yet started, and whether the provider is closing. A pump
// holds it, so a pump that runs after its provider is gone touches only this.
struct Queue {
    QMutex mutex;
    // The requests not yet started, oldest first.
    std::vector<std::unique_ptr<Job>> waiting;
    // Set when the provider closes: no request starts from then on (UI-213).
    bool closing = false;
};

// The newest job that has not started, or none: when they have all started, or the provider is closing.
std::unique_ptr<Job> take_newest(Queue& queue) {
    QMutexLocker lock(&queue.mutex);
    if (queue.closing || queue.waiting.empty()) return nullptr;
    std::unique_ptr<Job> job = std::move(queue.waiting.back());
    queue.waiting.pop_back();
    return job;
}

// One is queued on the pool for each request. When it runs, it takes the newest job that has not started yet, so the
// jobs start newest first. The pool orders these runnables all alike, so it needs no priority number, and none can wrap.
class Pump : public QRunnable {
public:
    explicit Pump(std::shared_ptr<Queue> queue) : m_queue(std::move(queue)) {}
    void run() override {
        if (std::unique_ptr<Job> job = take_newest(*m_queue)) job->run();
    }

private:
    std::shared_ptr<Queue> m_queue;
};

class Provider : public QQuickAsyncImageProvider {
public:
    explicit Provider(ImageKind kind) : m_kind(kind), m_pool(new QThreadPool), m_queue(std::make_shared<Queue>()) {
        // Thumbnails read originals and decode: enough threads to keep the disk queue full (8 in
        // flight). Full-size images are big; two at a time keeps memory in check. A Yandex picture waits on its file's
        // fetch: the grid asks for one at a time, and the preview for its current and two neighbouring originals, so four
        // threads are the most that wait at once.
        m_pool->setMaxThreadCount(kind == ImageKind::Thumb ? 8 : kind == ImageKind::Yandex ? 4 : 2);
    }

    ~Provider() override {
        {
            // Closing: no request that has not started starts, and none is started from now on (the pumps check it).
            QMutexLocker lock(&m_queue->mutex);
            m_queue->closing = true;
            m_queue->waiting.clear();
        }
        m_pool->clear();
        // The idle threads end now, not after their expiry: each one's thread-local connections close as it ends.
        m_pool->setExpiryTimeout(0);
        if (m_kind == ImageKind::Yandex) {
            // Not waited for (UI-213): the app has stopped Yandex, which cuts the fetches under way short at their next
            // read (Images::stop), but a download held in a read can't be woken from here, and the close must not wait for
            // it. The pool is left to end with the process; its pumps and jobs touch only the queue and the links they hold.
            return;
        }
        m_pool->waitForDone();
        delete m_pool;
    }

    QQuickImageResponse* requestImageResponse(const QString& id, const QSize&) override {
        auto link = std::make_shared<Link>();
        auto* response = new Response(link);
        {
            QMutexLocker lock(&m_queue->mutex);
            m_queue->waiting.push_back(std::make_unique<Job>(m_kind, id, link));
        }
        m_pool->start(new Pump(m_queue));
        return response;
    }

private:
    ImageKind m_kind;
    // Heap-allocated: a QThreadPool's destructor waits for its threads, and a Yandex pool must not be waited for at close.
    QThreadPool* m_pool;
    std::shared_ptr<Queue> m_queue;
};

}  // namespace

QQmlImageProviderBase* new_image_provider(ImageKind kind) { return new Provider(kind); }

}  // namespace pg
