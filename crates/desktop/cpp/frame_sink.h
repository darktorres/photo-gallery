// Frames from a Rust thread into a VideoOutput's QVideoSink (src/animation.rs).
#pragma once

#include <QtCore/QPointer>
#include <QtMultimedia/QVideoSink>

#include <cstdint>
#include <memory>

#include "rust/cxx.h"

namespace pg {

// The frames of one animation played. present() may be called from any thread: each frame is handed to the sink on
// the sink's thread, unless the sink is gone or close() was called by then.
class FrameSink {
public:
    explicit FrameSink(QPointer<QVideoSink> sink);
    void present(std::uint32_t width, std::uint32_t height, rust::Slice<const std::uint8_t> rgba) const;
    // On the sink's thread: frames still on their way are dropped.
    void close() const;

private:
    struct State {
        QPointer<QVideoSink> sink;
        bool open = true;
    };
    std::shared_ptr<State> state_;
};

// A weak reference to a sink, made and used on its thread.
class SinkRef {
public:
    explicit SinkRef(QVideoSink* sink) : sink_(sink) {}
    std::unique_ptr<FrameSink> frame_sink() const;

private:
    QPointer<QVideoSink> sink_;
};

std::unique_ptr<SinkRef> new_sink_ref(QVideoSink* sink);

}  // namespace pg
