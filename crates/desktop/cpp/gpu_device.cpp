// One D3D11 device for the app. Making one costs ~0.9 G instructions on this machine, most of it NVIDIA's loader
// scanning its DLL (PROFILING "The start, to ready"), and both the AV1 lookup and Qt Quick's renderer need one.
//
// The device is made the way Qt Quick makes its own (qrhid3d11.cpp, QRhiD3D11::create): the first adapter of a
// CreateDXGIFactory2 factory, D3D_DRIVER_TYPE_UNKNOWN, the default feature levels; plus VIDEO_SUPPORT for the
// decoder lookup. It is handed over only when Qt would have made a D3D11 device on that adapter with no debug layer,
// and only if it passes the checks Qt runs on its own (an imported device skips them).
//
// The GPU's AV1 decoder is looked up as FFmpeg's d3d11va hwaccel would: the AV1 Main profile, NV12 output (8-bit,
// which every AV1 decoder that has the profile also outputs). The settings go through qputenv: Qt reads them from
// the C runtime's environment, which Rust's set_var (SetEnvironmentVariableW) doesn't update.

#include "gpu_device.h"

#include <QtCore/QByteArrayView>
#include <QtCore/QLoggingCategory>
#include <QtCore/QtEnvironmentVariables>
#include <QtQuick/QQuickGraphicsConfiguration>
#include <QtQuick/QQuickGraphicsDevice>
#include <QtQuick/QQuickWindow>

#include <dxgi1_3.h>

#pragma comment(lib, "d3d11.lib")
#pragma comment(lib, "dxgi.lib")

namespace pg {

namespace {

// Off unless asked for, e.g. QT_LOGGING_RULES="pg.gpu.info=true".
Q_STATIC_LOGGING_CATEGORY(lc, "pg.gpu", QtWarningMsg)

using Microsoft::WRL::ComPtr;

// D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0. The SDK declares it with DEFINE_GUID, which needs dxguid.lib or INITGUID.
constexpr GUID av1_main = {0xb8be4ccb, 0xcf53, 0x46ba, {0x8d, 0x59, 0xd6, 0xb8, 0xa6, 0xda, 0x5d, 0x2a}};

bool decodes_av1(ID3D11Device* device) {
    if (qEnvironmentVariableIntValue("PG_FORCE_AV1")) return true;
    ComPtr<ID3D11VideoDevice> video;
    if (FAILED(device->QueryInterface(IID_PPV_ARGS(&video)))) return false;
    for (UINT i = 0, n = video->GetVideoDecoderProfileCount(); i < n; ++i) {
        GUID profile;
        if (SUCCEEDED(video->GetVideoDecoderProfile(i, &profile)) && profile == av1_main) {
            BOOL supported = FALSE;
            return SUCCEEDED(video->CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12, &supported)) && supported;
        }
    }
    return false;
}

// What Qt's create() requires of a device it makes: an 11.1 context, feature level 11_0 (shader model 5) and
// constant buffer offsetting. Null when the device has all of them, else the first one missing.
const char* missing_for_qt(ID3D11Device* device, ID3D11DeviceContext* context) {
    ComPtr<ID3D11DeviceContext1> context1;
    if (FAILED(context->QueryInterface(IID_PPV_ARGS(&context1)))) return "no ID3D11DeviceContext1";
    if (device->GetFeatureLevel() < D3D_FEATURE_LEVEL_11_0) return "feature level below 11_0";
    D3D11_FEATURE_DATA_D3D11_OPTIONS options = {};
    if (FAILED(device->CheckFeatureSupport(D3D11_FEATURE_D3D11_OPTIONS, &options, sizeof(options))) ||
        !options.ConstantBufferOffsetting)
        return "no constant buffer offsetting";
    return nullptr;
}

// Why Qt Quick should make its own device for `window`, or null when it can render with ours.
const char* keep_qt_device(QQuickWindow* window) {
    if (qEnvironmentVariable("PG_SHARED_DEVICE") == u"0") return "PG_SHARED_DEVICE=0";
    if (QQuickWindow::graphicsApi() != QSGRendererInterface::Direct3D11) return "Qt Quick doesn't render with D3D11";
    // Another adapter than the first: Qt reads it in QRhiD3D11::create, from the environment only.
    if (qEnvironmentVariableIsSet("QT_D3D_ADAPTER_INDEX")) return "QT_D3D_ADAPTER_INDEX is set";
    // From QSG_RHI_DEBUG_LAYER and QSG_RHI_PREFER_SOFTWARE_RENDERER, or set on the window.
    const QQuickGraphicsConfiguration config = window->graphicsConfiguration();
    if (config.isDebugLayerEnabled()) return "the D3D debug layer is on";
    if (config.prefersSoftwareDevice()) return "a software device is preferred";
    return nullptr;
}

}  // namespace

std::unique_ptr<GpuDevice> make_gpu_device() {
    auto gpu = std::make_unique<GpuDevice>();
    ComPtr<IDXGIFactory1> factory;
    ComPtr<IDXGIAdapter1> adapter;
    HRESULT hr = CreateDXGIFactory2(0, IID_PPV_ARGS(&factory));
    if (SUCCEEDED(hr)) hr = factory->EnumAdapters1(0, &adapter);
    if (SUCCEEDED(hr))
        hr = D3D11CreateDevice(adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                               nullptr, 0, D3D11_SDK_VERSION, &gpu->device, nullptr, &gpu->context);
    if (FAILED(hr)) {
        qCWarning(lc, "no D3D11 device (0x%08lx): no GPU decoding, and Qt Quick makes its own", hr);
        gpu->device.Reset();
        gpu->context.Reset();
        return gpu;
    }
    gpu->decodes_av1 = decodes_av1(gpu->device.Get());
    if (const char* missing = missing_for_qt(gpu->device.Get(), gpu->context.Get())) {
        qCWarning(lc, "the device can't render Qt Quick (%s): Qt makes its own", missing);
        gpu->device.Reset();
        gpu->context.Reset();
    }
    return gpu;
}

void choose_video_decoding(const GpuDevice& gpu) {
    // An empty list (Qt documents it as ","): none.
    constexpr const char* decoding = "QT_FFMPEG_DECODING_HW_DEVICE_TYPES";
    constexpr const char* encoding = "QT_FFMPEG_ENCODING_HW_DEVICE_TYPES";
    if (!qEnvironmentVariableIsSet(decoding)) {
        qputenv(decoding, gpu.decodes_av1 ? "d3d11va" : ",");
        qCInfo(lc, gpu.decodes_av1 ? "the GPU decodes AV1: videos decode on the GPU"
                                   : "the GPU doesn't decode AV1: videos decode on the CPU");
    }
    // Nothing is encoded through Qt Multimedia.
    if (!qEnvironmentVariableIsSet(encoding)) qputenv(encoding, ",");
}

void show_windows(QQmlApplicationEngine& engine, GpuDevice& gpu) {
    for (QObject* root : engine.rootObjects()) {
        auto* window = qobject_cast<QQuickWindow*>(root);
        if (!window) continue;
        // A device lost before a reload (debug builds) is never handed over again.
        if (gpu.device && FAILED(gpu.device->GetDeviceRemovedReason())) {
            qCWarning(lc, "the device was lost: Qt Quick makes its own");
            gpu.device.Reset();
            gpu.context.Reset();
        }
        if (gpu.device) {
            if (const char* reason = keep_qt_device(window)) {
                qCInfo(lc, "Qt Quick makes its own device: %s", reason);
                gpu.device.Reset();
                gpu.context.Reset();
            }
        }
        if (gpu.device) {
            qCInfo(lc, "Qt Quick renders with the app's device");
            // Read once, by the render thread when the window is first shown (QSGRhiSupport::createRhi). Qt keeps
            // no reference: ours are the ones that keep the device alive.
            window->setGraphicsDevice(QQuickGraphicsDevice::fromDeviceAndContext(gpu.device.Get(), gpu.context.Get()));
            // Once in use it's forgotten, on the thread that reads it: if the device is lost (a driver reset or
            // update), the renderer starts over with a device of Qt's own instead of the dead one.
            QObject::connect(
                window, &QQuickWindow::sceneGraphInitialized, window,
                [window] { window->setGraphicsDevice(QQuickGraphicsDevice()); },
                Qt::ConnectionType(Qt::DirectConnection | Qt::SingleShotConnection));
        }
        // Not QWindow::show: QQuickWindowQmlImpl keeps the QML property's state itself.
        window->setProperty("visible", true);
    }
}

}  // namespace pg
