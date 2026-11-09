/* Link and run check for the static FFmpeg build (tools/build-static-ffmpeg.ps1). It calls into each library, so the link
 * must resolve every symbol this level needs. Prints the library versions, the decoder lookups, the hardware device
 * types, and the result of opening a missing file. The script compares these lines with what it expects. */
#include <stdio.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/avutil.h>
#include <libavutil/hwcontext.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>

int main(void) {
    printf("libavutil   %u\n", avutil_version());
    printf("libavcodec  %u\n", avcodec_version());
    printf("libavformat %u\n", avformat_version());
    printf("libswscale  %u\n", swscale_version());
    printf("libswresample %u\n", swresample_version());
    printf("h264 decoder: %s\n", avcodec_find_decoder(AV_CODEC_ID_H264) ? "yes" : "no");
    printf("d3d11va hwdevice: %s\n",
           av_hwdevice_find_type_by_name("d3d11va") != AV_HWDEVICE_TYPE_NONE ? "yes" : "no");
    printf("dxva2 hwdevice: %s\n",
           av_hwdevice_find_type_by_name("dxva2") != AV_HWDEVICE_TYPE_NONE ? "yes" : "no");

    AVFormatContext *fmt = NULL;
    int ret = avformat_open_input(&fmt, "does-not-exist.mp4", NULL, NULL);
    printf("open missing file: %d\n", ret);
    return 0;
}
