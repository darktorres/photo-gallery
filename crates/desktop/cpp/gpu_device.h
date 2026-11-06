// The app's one D3D11 device on Windows: Qt Quick renders with it, and Qt's player is told how to decode on it
// (src/gpu_device.rs). Elsewhere there is no device of ours: Qt Quick makes its own, and videos decode on the CPU.
#pragma once

#include <memory>

#include <QtQml/QQmlApplicationEngine>

#ifdef Q_OS_WIN
#include <d3d11_1.h>
#include <wrl/client.h>
#endif

namespace pg {

// What was learned on the GPU, and on Windows its D3D11 device and immediate context.
struct GpuDevice {
#ifdef Q_OS_WIN
    // Null when Qt Quick makes its own (gpu_device.cpp says when).
    Microsoft::WRL::ComPtr<ID3D11Device> device;
    Microsoft::WRL::ComPtr<ID3D11DeviceContext> context;
#endif
    bool decodes_av1 = false;
};

// Make the device on the default adapter and look for the GPU's AV1 decoder on it. Any thread; it takes ~0.3 s.
// Never null: a device that can't be made, or that Qt Quick can't use, leaves only the AV1 answer.
std::unique_ptr<GpuDevice> make_gpu_device();

// Sets QT_FFMPEG_DECODING_HW_DEVICE_TYPES for this GPU, and QT_FFMPEG_ENCODING_HW_DEVICE_TYPES to none; a value
// already set is kept. Call it on the main thread before Qt Multimedia is first used.
void choose_video_decoding(const GpuDevice& gpu);

// Show the engine's windows (declared `visible: false`), rendering with the device when Qt Quick would make a D3D11
// device of its own; otherwise the device is let go. Call it after every load of the QML; the device must outlive
// the windows.
void show_windows(QQmlApplicationEngine& engine, GpuDevice& gpu);

}  // namespace pg
