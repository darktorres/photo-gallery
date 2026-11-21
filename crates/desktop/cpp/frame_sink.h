// Frames from a Rust thread into a VideoOutput's QVideoSink (src/animation.rs).
#pragma once

#include <QtCore/QPointer>
#include <QtMultimedia/QVideoSink>

#include <atomic>
#include <cstdint>
#include <memory>

#include "rust/cxx.h"

namespace pg {

// The frames of one animation played. present() and close() may be called from any thread: each frame is handed to the
// sink on the sink's thread, unless the sink is gone or close() was called by then. close() drops the frames still on
// their way; it only clears an atomic flag, which the sink's thread reads.
class FrameSink {
public:
    explicit FrameSink(QPointer<QVideoSink> sink);
    void present(std::uint32_t width, std::uint32_t height, rust::Slice<const std::uint8_t> rgba) const;
    void close() const;

private:
    // Built in place by make_shared: the atomic `open` makes State neither copyable nor movable.
    struct State {
        explicit State(QPointer<QVideoSink> s) : sink(std::move(s)) {}
        QPointer<QVideoSink> sink;
        std::atomic<bool> open{true};
    };
    std::shared_ptr<State> m_state;
};

// A weak reference to a sink, made and used on its thread.
class SinkRef {
public:
    explicit SinkRef(QVideoSink* sink) : m_sink(sink) {}
    std::unique_ptr<FrameSink> frame_sink() const;

private:
    QPointer<QVideoSink> m_sink;
};

std::unique_ptr<SinkRef> new_sink_ref(QVideoSink* sink);

}  // namespace pg
