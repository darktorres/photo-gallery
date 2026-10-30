// Async image providers: each request becomes a job on a thread pool, which calls Rust's load_image and hands
// the QImage back to the response on the GUI thread.
//
// Newest requests run first (LIFO by priority): while scrolling, the thumbnails now on screen are made before
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
#include <memory>

#include "pg-ui/src/providers.cxx.h"

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

QImage to_qimage(const Pixels& px) {
    if (px.data.empty()) return {};
    const auto* bytes = reinterpret_cast<const uchar*>(px.data.data());
    switch (px.format) {
        case 0:
            return QImage::fromData(bytes, int(px.data.size()));
        case 1:
            return QImage(bytes, px.width, px.height, px.width * 3, QImage::Format_RGB888).copy();
        case 2:
            return QImage(bytes, px.width, px.height, px.width * 4, QImage::Format_RGBA8888).copy();
        default:
            return {};
    }
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
    Job(int kind, QString id, std::shared_ptr<Link> link) : m_kind(kind), m_id(std::move(id)), m_link(std::move(link)) {}

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
    int m_kind;
    QString m_id;
    std::shared_ptr<Link> m_link;
};

class Provider : public QQuickAsyncImageProvider {
public:
    explicit Provider(int kind) : m_kind(kind) {
        // Thumbnails read originals and decode: enough threads to keep the disk queue full (FINDINGS "I/O": 8 in
        // flight). Full-size images are big; two at a time keeps memory in check.
        m_pool.setMaxThreadCount(kind == 2 ? 2 : kind == 1 ? 2 : 8);
    }

    ~Provider() override {
        m_pool.clear();
        m_pool.waitForDone();
    }

    QQuickImageResponse* requestImageResponse(const QString& id, const QSize&) override {
        auto link = std::make_shared<Link>();
        auto* response = new Response(link);
        m_pool.start(new Job(m_kind, id, link), m_priority.fetch_add(1));
        return response;
    }

private:
    int m_kind;
    QThreadPool m_pool;
    // Rising priority = last in, first out. Wraps after 2^31 requests, harmlessly.
    std::atomic<int> m_priority{0};
};

}  // namespace

QQmlImageProviderBase* new_image_provider(int kind) { return new Provider(kind); }

}  // namespace pg
