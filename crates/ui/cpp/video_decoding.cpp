// The GPU's AV1 decoder is looked up through D3D11, as FFmpeg's d3d11va hwaccel would: the default adapter (what
// av_hwdevice_ctx_create picks without a device name), the AV1 Main profile, NV12 output (8-bit, which every AV1
// decoder that has the profile also outputs).
//
// The settings go through qputenv: Qt reads them from the C runtime's environment, which Rust's set_var
// (SetEnvironmentVariableW) doesn't update.

#include "video_decoding.h"

#include <QtCore/QByteArrayView>
#include <QtCore/QtEnvironmentVariables>

#include <d3d11.h>
#include <wrl/client.h>

#pragma comment(lib, "d3d11.lib")

namespace pg {

namespace {

// D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0. The SDK declares it with DEFINE_GUID, which needs dxguid.lib or INITGUID.
constexpr GUID av1_main = {0xb8be4ccb, 0xcf53, 0x46ba, {0x8d, 0x59, 0xd6, 0xb8, 0xa6, 0xda, 0x5d, 0x2a}};

bool gpu_decodes_av1() {
    using Microsoft::WRL::ComPtr;
    ComPtr<ID3D11Device> device;
    if (FAILED(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                                 nullptr, 0, D3D11_SDK_VERSION, &device, nullptr, nullptr)))
        return false;
    ComPtr<ID3D11VideoDevice> video;
    if (FAILED(device.As(&video)))
        return false;
    for (UINT i = 0, n = video->GetVideoDecoderProfileCount(); i < n; ++i) {
        GUID profile;
        if (SUCCEEDED(video->GetVideoDecoderProfile(i, &profile)) && profile == av1_main) {
            BOOL supported = FALSE;
            return SUCCEEDED(video->CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12, &supported)) && supported;
        }
    }
    return false;
}

}  // namespace

void choose_video_decoding() {
    // An empty list (Qt documents it as ","): none.
    constexpr const char* decoding = "QT_FFMPEG_DECODING_HW_DEVICE_TYPES";
    constexpr const char* encoding = "QT_FFMPEG_ENCODING_HW_DEVICE_TYPES";
    if (!qEnvironmentVariableIsSet(decoding))
        qputenv(decoding, gpu_decodes_av1() ? "d3d11va" : ",");
    // Nothing is encoded through Qt Multimedia.
    if (!qEnvironmentVariableIsSet(encoding))
        qputenv(encoding, ",");
}

}  // namespace pg
