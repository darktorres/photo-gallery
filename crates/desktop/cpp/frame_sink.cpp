#include "frame_sink.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QMetaObject>
#include <QtGui/QImage>
#include <QtMultimedia/QVideoFrame>

namespace pg {

FrameSink::FrameSink(QPointer<QVideoSink> sink) : m_state(std::make_shared<State>(std::move(sink))) {}

void FrameSink::present(std::uint32_t width, std::uint32_t height, rust::Slice<const std::uint8_t> rgba) const {
    const qsizetype line = qsizetype(width) * 4;
    if (width == 0 || height == 0 || qsizetype(rgba.size()) < line * height)
        return;
    // Copied: the Rust buffer is gone once this returns.
    QVideoFrame frame(QImage(rgba.data(), int(width), int(height), line, QImage::Format_RGBA8888).copy());
    // The sink and `open` are only touched on the application's (the sink's) thread.
    QMetaObject::invokeMethod(
        QCoreApplication::instance(),
        [state = m_state, frame] {
            if (state->open.load() && state->sink)
                state->sink->setVideoFrame(frame);
        },
        Qt::QueuedConnection);
}

void FrameSink::close() const {
    m_state->open.store(false);
}

std::unique_ptr<FrameSink> SinkRef::frame_sink() const {
    return std::make_unique<FrameSink>(m_sink);
}

std::unique_ptr<SinkRef> new_sink_ref(QVideoSink* sink) {
    return std::make_unique<SinkRef>(sink);
}

}  // namespace pg
