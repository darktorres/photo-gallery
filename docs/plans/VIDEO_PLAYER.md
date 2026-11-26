# Video player: options (2026-11-14, not decided)

The viewer plays videos with Qt Multimedia today. That is a stopgap. The section "Videos and animations (M2)" at the end of this page has the measurements and the web research behind it. In short:

- On this RTX 2060, Qt's FFmpeg backend plays AV1 as 0 frames. Qt picks the decoder by device type and pixel
  format, with no check of the GPU, and opens FFmpeg's native `av1` decoder. That decoder can only use the
  GPU: the Turing GPU has no AV1 decode profile, so FFmpeg's hardware setup fails on every frame, and there is no software path
  to fall back to. `libdav1d` would have decoded it.
- The stopgap turns hardware decoding off for every codec whenever the GPU lacks AV1. It does this through
  the `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` environment variable, set from C++ because Qt reads the CRT
  environment.
- Animations already have their own player: `AnimationPlayer`, which uses image-crate decoders and pushes
  frames into a `QVideoSink`. FFmpeg's animated WebP demuxer has no seek, so it can't loop.

Two replacements are on the table. Both keep the integration in our hands, with no third-party wrapper
libraries (Option A uses the raw `ffmpeg-sys-next` bindings); they differ in who owns the playback core. Both were checked against the sources
(see "Validation"); this page includes the corrections.

## What the player has to do

- Local files only: the scanned containers (mp4, m4v, mov, webm, mkv, avi, wmv, mpg/mpeg, 3gp) with the
  codecs FFmpeg decodes.
- Decide per file between hardware and software decoding. Hardware when the GPU supports the codec,
  profile, size and bit depth; software (libdav1d for AV1) otherwise, or when hardware fails at run time.
- Autoplay with sound, loop without a gap, pause, exact seek, frame step, mute and volume.
  - Step back is in use: Left and Backspace (`crates/client/src/keys.rs`), the wheel and a horizontal swipe
    (`Viewer.qml`). It costs a re-decode from the previous keyframe, so the player must support it.
- The video's own orientation (the display matrix, as phones write it) plus the catalog turn, applied on
  display. Qt applies the display matrix today, so dropping Qt means doing it ourselves. Fitted by default, with the
  zoom PLAN asks for (fit, 1:1, continuous zoom, 10× loupe).
- Draw inside the Qt Quick scene, so QML controls can sit on top of the video. A native child window can't
  do that.
- Correct colours: BT.601/709/2020 matrices, limited and full range, 8- and 10-bit. HDR tone-mapped to
  SDR. The output stays SDR in v1: an HDR swapchain is per window, so it would change the whole gallery UI.
- Open fast when stepping through a folder. Ideally the neighbouring videos are opened ahead of time.
- Licence: the FFmpeg we ship and test is BtbN's GPL shared build (`gpl-shared`, see `.github/workflows/ci.yml`),
  linked dynamically. The licence texts and source offer must ship with the DLLs (none are shipped yet, for FFmpeg
  or Qt). Nothing of ours on C:.

## Shared groundwork (needed by both)

- **One D3D11 device for Qt and the decoder.**
  - We create it at startup on one chosen adapter, with `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`, and will turn on
    multithread protection with `ID3D10Multithread::SetMultithreadProtected`. Neither Qt nor FFmpeg turns
    that on for a device it is given (FFmpeg does only for devices it creates itself).
  - Qt Quick's default Windows backend is Direct3D 11 (checked at `gpu_device.cpp:72`), and we hand the device over with
    `setGraphicsDevice(QQuickGraphicsDevice::fromDeviceAndContext(...))`, public API. It must happen
    before the scene graph initialises (a later call only takes effect when the RHI is recreated), so the window is declared hidden and
    shown after the load, once it has the device. Check it worked through
    `rendererInterface()` (or the `qt.scenegraph.general` log line "Using existing native D3D11 device", which is debug-only: enable the category or `QSG_INFO=1`).
  - Qt borrows the device and context without taking a reference. Our owner object keeps them alive until
    the window and every decoder are gone.
  - No copies between devices, but one copy on the same device per decoded frame: the decoder reuses its
    texture slots, so each frame is copied out into a texture of ours.
  - **Qt's half is done** (`crates/desktop/cpp/gpu_device.cpp`): the device is made on the first
    adapter, the AV1 check runs on it, and the window, declared hidden, is handed it before it's shown.
    Multithread protection isn't on yet: nothing but Qt's render thread uses the device so far.
  - Device loss: with the device left set, Qt would keep retrying with the dead one. So the hand-over is
    forgotten once the scene graph is up (`sceneGraphInitialized`, on the render thread), and after a loss Qt
    makes a device of its own. The decoder will need the same: follow Qt's device, or get a new one of ours.
  - Qt's own FFmpeg backend works differently: a device of its own per file, and three copies per frame
    one from FFmpeg's pool to a same-device texture, then two through a shared texture with a keyed mutex. That design is the fallback if one shared device stalls.
- **A `QQuickRhiItem` (public since Qt 6.7)** that draws the video into the scene.
  - Written in hand-written C++ (a header with `Q_OBJECT` and `QML_ELEMENT`, run through moc by
    `cxx-qt-build`'s `cpp_file`). Its renderer is not a QObject, so CXX-Qt can't subclass it.
  - Its hooks run on the render thread. `initialize()` runs on every sync, so it must be idempotent, and the
    colour texture can be recreated on resize, so its native handle is read every frame.
  - Raw D3D11 work in `render()` goes between `QRhiCommandBuffer::beginExternal()` and `endExternal()`, inside
    a render pass the item opens with `ExternalContent`. `beginExternal()` runs Qt's recorded commands first,
    and `endExternal()` makes Qt rebind its state.
  - It also draws our animation frames, replacing `QVideoSink` + `VideoOutput`.
  - With the software scene graph (`QT_QUICK_BACKEND=software` or `setGraphicsApi(Software)`) it draws nothing,
    so its tests need a real window.
- **Qt Multimedia goes away, in stages.** With it go Qt's FFmpeg plugin, the environment-variable stopgap
  (`choose_video_decoding` in `crates/desktop/cpp/gpu_device.*` and its call in `crates/desktop/src/lib.rs`), `windeployqt --no-ffmpeg`
  and `frame_sink.*`.
  Until the new player has sound, Qt Multimedia stays: it is today's only audio path.
- **The transport bar and keys stay in QML** and drive the item through properties and invokables. The seek
  bar sends a seek on every drag move, so the player coalesces seeks.
- **Licences (not built yet):** the deploy step will write a licences folder (LGPL and GPL texts, a notice per bundled library,
  the FFmpeg configure line and a source offer) and the app will get an About entry.

## Option A: our own playback engine on our FFmpeg 9

We write the player on top of the FFmpeg 9.0 we already pin (BtbN `gpl-shared`, `FFMPEG_DIR`).

References, for design ideas only:

- Qt Multimedia's FFmpeg backend: the playback engine, demuxer, stream decoder, video and audio renderers,
  time controller, and its D3D11 hardware-acceleration code;
- ffplay.c, a complete FFmpeg player in about 4.7k lines of C (with no software fallback);
- mpv's hardware-decoding fallback logic.

### A: pieces

- **Bindings.** A thin safe layer of our own over `ffmpeg-sys-next`, covering what the player uses.
  - `ffmpeg-next` has no hardware-decoding API, and its README claims support only up to FFmpeg 8.0.
    `ffmpeg-sys-next` binds the D3D11 device and frames contexts when the Windows SDK `d3d11.h` is on bindgen's include path.
  - Missing from its bindings: `dovi_meta.h` and `mastering_display_metadata.h` (for HDR and Dolby Vision),
    and the D3D11 constants. We add those.
  - Frame and packet wrappers are `Send` but not `Sync`, and hardware frames are shared with `av_frame_ref`,
    never deep-copied.
  - Enable swresample (`software-resampling`) for audio.
- **Threads and queues.** Demux → video decode and audio decode.
  - Bounded queues: packets by time and bytes (Qt uses 4 s and 32 MB), decoded frames by count (Qt uses 3
    for video and 9 for audio).
  - A session id on every packet and frame drops stale data after a seek or loop, as Qt and ffplay do.
  - Pause stops the renderers only; the decoders keep filling their queues, which makes stepping forward
    cheap.
- **Clock.** Audio is the master clock when there is audio. 1,080 of the 1,712 dataset videos have no
  audio, so the steady-clock path is the common case, not an edge case.
  - The heard position is frames written minus `IAudioClock::GetPosition` (the sample now at the speakers),
    extrapolated with the QPC time `GetPosition` returns, as mpv, Chromium and Firefox do. Padding and
    `GetStreamLatency` are not subtracted on top (that counts the delay twice; `GetStreamLatency` is a buffer
    size, not the output delay). PTS is tagged after swresample, which accounts for its delay once. `GetFrequency`
    is constant over a stream's life. The docs don't quantify the residual error, so the A/V offset gets an
    acceptance criterion (see "Research 2026-11-25 → Audio clock").
  - The clock starts when the first video frame and first audio buffer are ready (Qt starts it on the first
    packet, so its first frames come late and render back to back).
  - At each render tick the newest frame due is shown and older ones are dropped. Qt has no late-frame drop.
- **Hardware decoding:**
  - **Choose the decoder by name.** `av1` with the hardware device when the GPU decodes AV1, `libdav1d`
    otherwise; never by codec id, whose order isn't guaranteed.
  - **Before opening.** A table maps codec, profile, bit depth and chroma to the D3D11 decoder GUID and
    output format (NV12 or P010). It is checked once on the shared device with `CheckVideoDecoderFormat`,
    and per stream with `GetVideoDecoderConfigCount`, passing the frame size in the descriptor (`SampleWidth`/`SampleHeight`). VP8 is always software: this
    FFmpeg build has no VP8 D3D11 decoder.
  - **`get_format`** returns the hardware format only if the stream passed the table; it never panics across
    the FFI and handles being called again after a failed format. Set `AV_HWACCEL_FLAG_IGNORE_LEVEL`, never
    `ALLOW_PROFILE_MISMATCH`.
  - **At run time.** Fall back after 3 consecutive decode errors (mpv's rule), or when a watchdog sees no
    output frames. Errors are classified: `EAGAIN` and `EOF` are flow control, anything else counts. The
    fallback reopens in software, seeks to the keyframe before the current position and decodes up to it.
    mpv doesn't do that (it only replays packets before the first frame), so this part is ours.
  - **Pool.** `extra_hw_frames` covers the frames we hold; the number of open hardware decoders is capped
    at a fixed count of our choosing (NVIDIA documents no decode-session limit; VRAM and the one NVDEC engine
    are the real limits). A failed `CreateVideoDecoder` for a neighbour means software or later, not an error.
- **Rendering.**
  - **Hardware frames:** at receive time, copy the decoder's slice (one `CopySubresourceRegion` moves both
    planes, as Qt's shipped code does) into a ring of 2–3 NV12 or P010 textures of ours, created once. A ring
    slot is reused only after the frame that sampled it has reached `frameSwapped` (QRhi replays its recorded
    commands at `endFrame`, so a copy before that would land ahead of the draw).
  - **Planes:** each texture is wrapped once per plane, R8 + RG8 (NV12) or R16 + RG16 (P010), as Qt does.
    QRhi has no NV12 format.
  - **Software frames:** upload the planes and use the same shaders. Animation frames upload as RGBA.
  - **Shaders:** the kit has no `qsb`, but the official 6.12.0 one is the Qt Shader Tools addon
    (`aqt install-qt windows desktop 6.12.0 win64_msvc2022_64 -m qtshadertools`, or MaintenanceTool, or the
    PySide6-Addons wheel). Qt Creator's qsb 6.11.2 also works: both write QSB format 9 and produced
    byte-identical files. Bake `--hlsl 50` (the only HLSL version the D3D11 backend looks up). `build.rs` runs
    it and embeds the `.qsb` files; with `-c` it must put `fxc` on PATH and check the output with `qsb -d`,
    because a missing fxc still exits 0 and silently stores HLSL source.
- **Colour.** Each stage is its own step with a unit test:
  1. Matrix and range from the tags, with one rule for untagged video shared with the importer (Qt
     assumes BT.709 above 576 lines on its GPU path, our importer at 720 and up). Adopt mpv's rule,
     `width >= 1280 || height > 576` → BT.709, in both: the importer's rule treats 1280×544 or 1920×800 as
     BT.601. Also Chromium's quirk: a GBR matrix tag on non-4:4:4 H.264/HEVC means BT.709.
  2. BT.2020 to BT.709 primaries for every BT.2020 source, not only HDR.
  3. Transfer decode chosen by the transfer function, not the bit depth. The dataset has three 8-bit HLG
     files; Qt sends those through its plain SDR shader.
  4. Tone mapping, gamut clip, SDR encode.
- **HDR.** Our own BT.2390 tone mapping, specified from the standard (the EETF text now lives in
  BT.2408-7 Annex 5), not ported from Qt. The SDR target peak is 203 nits (1.0 = 203, as mpv and libplacebo),
  not 100: with 100, a 1000-nit source's reference white lands at 88 nits; with 203, at about 90 % signal.
  Applied as maxRGB. The shader-ready chain is in "Research 2026-11-25 → HDR and colour".
  - Qt's version has bugs: its target luminance isn't normalised to the source's peak (a 1000-nit HDR10 source
    peaks at about 27 nits instead of 100; with no metadata Qt assumes 10000 nits and is right at the peak), its HLG path uses a fixed gamma-1.2 OOTF that ignores the peak luminance, and software-decoded 10-bit
    frames get no tone mapping at all. Its full-range BT.601 matrix has the wrong R, G and B coefficients (R uses 1.772 for 1.402, B uses 1.42 for 1.772, and G uses the BT.2020 terms).
  - HLG gets the BT.2100 OOTF at Lw = 1000 (gamma 1.2) to display light, then the same EETF to 203 nits: one
    path for PQ and HLG, as mpv and libplacebo do. Lw = 100–203 directly would put reference white near 58 nits.
    The source peak comes from the mastering metadata, then MaxCLL, then 1000 nits (Kodi's default; mpv and
    libplacebo assume 10000 for PQ, which dims typical masters), with a sanity range of 100–10000. Dynamic peak
    detection is v2.
  - Dolby Vision: profiles 8.1, 8.4 and 7.6 play correctly as their base layer (HDR10 or HLG), keyed on the base
    layer's own colour tags. The configuration record (`AV_PKT_DATA_DOVI_CONF`) is used only to detect profile 5
    (compatibility id 0), which shows wrong colours without reshaping. Ids: 0 = profile 5, 1 = HDR10, 2 = SDR,
    4 = HLG. Profile 5 needs reshaping plus the RPU's matrices (FFmpeg already attaches
    `AV_FRAME_DATA_DOVI_METADATA` per frame): about 300–500 lines of ours. Porting libplacebo's code makes that
    module LGPL-2.1+, so write it from FFmpeg's `dovi_meta.h`; Dolby patents are a separate question. Samples now
    exist (Jellyfin test videos), so it can be in v1 if wanted. This FFmpeg build has the libplacebo filter (with
    `apply_dolbyvision`), a possible route that needs Vulkan.
- **Orientation.** The display matrix comes from `codecpar->coded_side_data` (FFmpeg 9 has no stream side
  data), composed with the catalog turn into one EXIF-style orientation, applied as a UV transform in the
  shader. The mirror mapping is `matrix_orientation` in `crates/video/src/lib.rs`, covered by a unit test of the
  eight matrices ffmpeg writes.
- **Audio.**
  - WASAPI directly (the `windows` crate for COM), shared and event-driven, with the mix format and its
    channel mask, and MMCSS registration.
  - **Gain:** one float gain stage, with a short ramp on each change; mute is gain 0 with the stream still
    running, so the clock keeps counting.
  - **Device change:** a debounced default-device notification or `AUDCLNT_E_DEVICE_INVALIDATED` stops,
    rebuilds the client and swresample for the new format, and re-anchors the clock at the current media
    time. Video keeps going on the steady clock meanwhile.
- **Transport.**
  - Exact seek: seek to the keyframe before the target, decode and discard up to it, and trim the first audio
    frame.
  - Loop: at the end, drain the decoder, then flush it, so the last frames of each pass aren't lost.
  - Frame step forward: show the next decoded frame.
- **Preloading.** Neighbouring videos are opened and decoded up to their first frame while the current one
  plays, within the hardware-decoder cap, and dropped when you move away.

### A: size and risk

- An estimate, not measured: roughly 6–8k lines of Rust plus the C++ item and its shaders. For scale, Qt's playback engine
  (`qtmultimedia/src/plugins/multimedia/ffmpeg/playbackengine`) is about 4.1k lines of C++.
- **Main risks:**
  - One shared device: decoding and Qt's rendering on one immediate context. From the sources, multithread
    protection plus one FFmpeg device context is enough for correctness (mpv works this way), provided only
    Qt's render thread sets pipeline state and ring slots wait for `frameSwapped`. The spike measures
    throughput; a frame lock (`afterRendering` → `frameSwapped`) would stall decoding only for Qt's replay plus
    `Present`, not a vsync. See "Research 2026-11-25 → Shared device".
  - A/V sync on messy files (variable frame rate, broken timestamps, MP4 edit lists) and the accuracy of the
    audio clock.
  - Audio devices changing or disappearing mid-play.
  - HDR quality relative to libplacebo, and Dolby Vision profile 5.
  - Bugs only show on files we haven't seen. The dataset has only H.264, VP9 and VP8, 8-bit, unrotated, and
    stereo or mono AAC; the rest has to be generated or found.

### What A gives over B

- **One FFmpeg, ours,** used for import, thumbnails and playback.
- **Full control over gallery behaviour:**
  - preloading neighbours;
  - what stepping and looping mean;
  - instant first frame;
  - one render path for videos and animations.
- **No extra toolchain or runtime.**

## Option B: libmpv, integrated by us

We embed mpv's player core through libmpv. We pin an mpv master commit for the D3D11 render API (mpv
PR #17764, merged on 2026-10-15 into the 0.42.0 milestone, which shipped on 2026-11-05). The pin must be after
PR #18845 (merged 2026-11-22, to ship in 0.43.0, client API 2.7), which brings gpu-next into the render API
(`MPV_RENDER_PARAM_RENDERER`; gpu-next is then the default) and needs libplacebo built with D3D11.

### B: pieces

- **Bindings.** Our own `bindgen` over `client.h`, `render.h` and `render_d3d11.h` (53 functions across `client.h` and `render.h`; `render_d3d11.h` declares only structs; the player
  needs a subset of them, through an allowlist; the subset is set when the bridge is written). No `libmpv2` or other crates. Check `mpv_client_api_version()`
  for 2.6 or later at start.
- **Render bridge.**
  - A C++ bridge object, not the item, owns the mpv core and its render context (one context per core).
    The context must exist before the first `loadfile`, and is freed on the render thread, outside any pass,
    before the device is released.
  - mpv takes our device through `MPV_RENDER_PARAM_D3D11_INIT_PARAMS` and adds its own reference.
  - In the item's `render()`: begin a pass on the item's render target with `ExternalContent`, then
    `beginExternal()`, then `mpv_render_context_render` into the item's colour texture (RGBA8 or RGB10A2,
    one sample), then `endExternal()` and end the pass. mpv renders on the immediate context and restores no
    state; `endExternal()` makes Qt rebind its own.
  - `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME` = 0, pacing from the frame's target time. The bridge must then wait for the target time itself, or A/V sync drifts (`render.h:308-318`). Otherwise each frame
    blocks Qt's render thread for up to `video-timing-offset` (50 ms by default).
  - mpv's update callback only posts a queued call to the GUI thread, which calls `update()`. Every
    `mpv_render_*` call stays on the render thread; control calls use the async API.
  - `mpv_render_context_report_swap` from `QQuickWindow::frameSwapped`.
- **Hardware decoding.** d3d11va decodes on our device from mpv's decoder thread, and mpv turns on
  multithread protection itself (so our device must not be created single-threaded). By default that is one copy per
  frame on the same device; zero-copy is an option with documented driver caveats.
  - `hwdec=d3d11va` (`auto` and `auto-safe` are the same, and pick the decoder themselves), with `hwdec-codecs` set
    per file from our AV1 check on the shared adapter. The manual lists d3d11va for `vo=gpu` and `gpu-next` only. The code needs just a D3D11 render context, which the render API provides, and it does: on this machine (0.41 build, `hwdec-current` = `d3d11va`, frames rendered), H.264 and VP9 decode through d3d11va, and 8-bit 4:2:0 HEVC Main does too (tested on a synthetic clip only). AV1 does not on this Turing GPU: the first frame fails, and mpv falls back to software for the rest of the clip, so playback works without hardware decode.
  - mpv falls back to software after 3 consecutive decode errors (`hwdec-software-fallback`). A watchdog of ours
    reopens with `hwdec=no` when the decoder produces no frames.
- **Control.** Commands and properties go through the client API:
  - `loadfile`, `seek <t> absolute` (exact by default; `absolute+keyframes` while dragging), `frame-step`,
    `frame-back-step` (dropped while an exact seek is settling; keyframe seeks don't block it), `loop-file=inf`, `pause`, `mute`, `volume`,
    `video-rotate` for the catalog turn (frames already come out upright by the display matrix, mirror
    included: mpv takes a vertical flip from the frame matrix's determinant and flips in the shader). Pass
    `video-rotate` as a `loadfile` per-file option: changing it at run time resets the decoder and seeks.
    Never `vf=hflip` (a software filter that forces a copy back).
  - Seeks and steps wait for `MPV_EVENT_FILE_LOADED`. Change events come back through
    `mpv_observe_property` and map to Qt properties.
- **Options:** `vo=libmpv`, `config=no` (no user config or scripts), `load-scripts=no`, `osd-level=0` and
  `osd-on-seek=no` (mpv draws its OSD into the video by default), `audio-display=no`, `keep-open=always`,
  `resume-playback=no`, `audio-device=auto`, `tone-mapping` (works on this renderer), and
  `gpu-shader-cache-dir` on D: (by default the system cache directory).
  - The render API gives mpv no output colour space, so mpv assumes BT.709 output. `target-prim`, `target-trc`
    and `target-peak` are ours to set and override that. `target-colorspace-hint` needs gpu-next, which the
    render API does not use.
- **Audio.** mpv's WASAPI output requests a reload on a default-device change, unplug or format change (`ao_request_reload`).
  mpv reopens the device on the next audio fill. If the reopen fails, it tries the other audio drivers, then switches to another audio track if one exists, and otherwise drops
  the track with no retry, so we watch the track list, re-select the audio track when a device is back, and show a
  "no sound" state.
- **Building libmpv.** Two ways:
  1. **We build it** (the shipping route). Pinned mpv commit, meson with MSYS2 clang64 under D:, against our
     FFmpeg 9 DLLs (BtbN ships the MinGW `.dll.a` import libraries): one FFmpeg.
     - Dependencies beyond libplacebo and libass (freetype, fribidi, harfbuzz): shaderc and spirv-cross (the
       D3D11 renderer needs both at configure time), Vulkan headers and lcms2 are optional: configure passes with `-Dvulkan=disabled` and with `-Dlcms2=disabled`
       (tested on MSYS2). With `-Dvulkan=disabled`, `vulkan-1.dll` stays in the closure, since libplacebo still imports it (measured). mpv also needs FFmpeg's avfilter DLL
       (required; avfilter-12.dll, 37.7 MB, is not deployed today). avdevice is optional: it is linked only if its
       feature is on.
     - Configure (tested on MSYS2 clang64, meson 1.12.1, mpv 0.42.0):
       `meson setup build src -Dbuildtype=release -Dgpl=false -Dlibmpv=true -Dcplayer=false -Dauto_features=disabled
       -Dgl=disabled -Dwin32-threads=enabled -Dd3d11=enabled -Dshaderc=enabled -Dspirv-cross=enabled -Dlcms2=enabled
       -Dvulkan=enabled -Drubberband=disabled`. `auto_features=disabled` is what turns every optional feature off;
       `-Dgl=disabled` is needed because gl otherwise fails configure without an OpenGL output; `-Dwin32-threads=enabled`
       is needed because the pthread timer path does not compile on this toolchain. d3d11 needs shaderc and spirv-cross,
       and configure fails without them, so a missing one is caught. `-Dgpl=false` does not gate rubberband (its
       meson file has no gpl check), so it is disabled by name.
     - The runtime, measured on the MSYS2 build: 23 MSYS2 DLLs (26.1 MB: libc++, libplacebo, shaderc, spirv-cross,
       libass and its font stack), `vulkan-1.dll` (0.76 MB, imported by libplacebo and by libmpv itself), and six of our
       FFmpeg DLLs (183.7 MB: avcodec, avfilter, avformat, avutil, swresample, swscale). The deploy copies an explicit
       list, checked against the built DLL's imports. The clang64 `vulkan-1.dll` must be copied, not the System32 one.
     - Don't install MSYS2's mpv or ffmpeg packages: they are GPL builds, and their FFmpeg DLLs have the
       same names as ours.
  2. **A pinned daily LGPL build** (zhongfly/mpv-winbuild), for the spike only. Pin its sha256 and keep a
     copy on D:; the release tags are pruned and can be replaced. It lacks `render_d3d11.h` (take it from the
     same mpv commit), an MSVC `.lib` (generate one from the exports, or load the DLL at run time) and
     licence texts. It links its own FFmpeg statically (a second, different FFmpeg 9) and imports
     `vulkan-1.dll`.
- **Animations.** These stay on our own decoders and upload through our side of the item. mpv reads
  animated WebP through the same FFmpeg demuxer, which has no seek and cannot loop. Our other threads must not use the
  immediate context while mpv renders.

### B: size and risk

- Our side: the C++ item and bridge, the bindings, a QML adaptor, the audio and decoder watchdogs, and the
  build and deploy of libmpv (way 1). An estimate from comparable integrations (Memento's D3D11
  `QQuickRhiItem` renderer is 330 lines, its controller about 3.1k; KDE MpvQt about 1.2k): roughly 2.3–3.6k
  lines in all.
- **Main risks:**
  - The D3D11 render API is new (mpv 0.42.0) and was merged on 2026-10-15 (kasper93). The maintainer calls the render
    API "limited" and suggests window embedding for D3D11 instead (2026-04-18, on PR #17764, before gpu-next
    reached the render API). Device loss isn't reported on D3D11, and open PR #18700 wouldn't change that: it
    touches only the windowed contexts, not `vo_libmpv.c`, and is stalled. We detect loss through Qt and DXGI;
    freeing the render context fails the video track, so recovery is a new device and context plus a reload at
    the current position. Bugs in it are ours to work around until fixed upstream; gpu-next in the render API
    is days old (#18903 open).
  - Open mpv bugs that touch our design: #18873 (a mid-stream hardware fallback drops frames until the next
    keyframe) and #18823 (WASAPI doesn't recover when a device returns).
  - Way 1's toolchain and runtime: MSYS2 upkeep (the build needs 64 clang64 packages) and a 23-DLL MSYS2 closure plus
    our FFmpeg DLLs on every mpv bump.
  - Opening time and preloading. Playlist prefetch starts only at the current file's EOF, next file only,
    and never decodes; a pre-decoded neighbour needs a second mpv core with its own render context, which is
    supported (each core turns on protection on our device and shares the immediate context). Open preloaded
    cores with `aid=no`. Time to first frame is dominated by the first shader compile (cached in
    `gpu-shader-cache-dir`), decoder creation and WASAPI open; mp4 and mkv skip stream probing.
  - The loop is `loop-file=inf`, an end-of-file seek back to the start that resets decoders and waits for the
    first new frame to be shown. From the code, not measured: the last frame holds about 1–2 display frames
    too long at each wrap. `cache=yes` and `video-latency-hacks=yes` shorten it; only `vf=loop` is seamless,
    and it keeps every frame in RAM.
  - Less control: mpv's behaviour comes through its options. Anything mpv doesn't expose, we can't change.

### What B gives over A

- **The most battle-tested playback core,** for less code of ours: sync, seeking, frame stepping, looping,
  per-file fallback and audio device changes.
- **The widest tolerance for odd files.**
- **HDR from libplacebo, with a newer pin.** At 0.42.0 the render API runs mpv's older renderer (vo_gpu's
  `gl_video`: BT.2390, Dolby Vision as base layer only). PR #18845 (2026-11-22) closed issue #10810 and
  puts gpu-next in the render API, so a master pin gets libplacebo's tone mapping, gamut mapping and Dolby
  Vision reshaping (profile 5 shown working by a tester, on macOS). The output colour space is still fixed,
  so `target-*` stays ours.

## Comparison

|                             | A: own engine                                       | B: libmpv                                                               |
|-----------------------------|-----------------------------------------------------|-------------------------------------------------------------------------|
| Our code                    | ~6–8k lines + C++ item and shaders                  | ~2.3–3.6k lines: item, bridge, bindings, watchdogs, mpv build           |
| FFmpeg copies               | one (ours)                                          | one (way 1) or two (way 2)                                              |
| Toolchain and runtime added | qsb (Creator's, or built once)                      | MSYS2 + meson, 23 MSYS2 DLLs + vulkan-1 + 6 FFmpeg DLLs, 210 MB (way 1) |
| Hardware fallback           | ours: table, decoder by name, 3 errors, watchdog    | mpv's, plus our AV1 check and a watchdog                                |
| HDR                         | our BT.2390 + HLG OOTF; DV 8.x as base layer; P5 ~300–500 lines more | gpu-next (master pin): libplacebo, DV reshaping; 0.42.0: gl_video |
| Audio device changes        | ours                                                | mpv's, plus a re-select when reopening fails                            |
| Odd files                   | depends on us                                       | mature                                                                  |
| Gallery-specific control    | full                                                | through mpv options                                                     |
| Neighbour preloading        | built in                                            | a second core per neighbour                                             |
| Animations                  | same pipeline                                       | our decoders, our upload                                                |
| Upstream risk               | FFmpeg API only                                     | new (0.42.0) render API; gpu-next in it days old; no device-loss report |

## Validation (2026-11-14)

Each option was checked against the sources: Qt 6.12's source, FFmpeg 9's headers and DLLs, the Windows SDK,
mpv and libplacebo master, PR #17764, a prebuilt libmpv, the crates and the dataset (a header-only ffprobe of
all its videos). Claims were checked against the cited lines and the sources they name. The corrections are in the sections above;
the ones that moved the comparison:

- **A:** the AV1 root cause (the decoder choice, not a missing check), thread safety on one shared device,
  the ring of copied frames, the missing orientation and audio-gain designs, no `qsb` in the kit, Qt's HDR
  bugs, and the size (6–8k lines, not 4–6k).
- **B:** the HDR advantage is gone (the render API doesn't use libplacebo's renderer), `gpu-next` isn't an
  option there, the render step needs the external-content bracket, way 1 needs shaderc, spirv-cross and a
  23-MSYS2-DLL runtime (plus our FFmpeg DLLs), and way 2 is a spike artifact only.
- **Both:** the device must be handed to Qt before the window is shown, Qt can't recover a lost imported
  device, the item is hand-written C++, and nothing ships licence texts yet.

## Spikes

Run the chosen option in this order, hardest part first, and stop at the first step that fails badly. The
first step of each option is the decision test. Neither spike yet covers device loss, so add a driver-reset check to step 1 of both.

**Option A:**

0. Bake one NV12 shader with qsb and load it in the 6.12 runtime. Mostly settled by the research (same
   format, byte-identical output, qsb 6.12 reads it); what remains is drawing with it in the app.
1. The shared device (attached before the window shows), the `QQuickRhiItem`, decoding with the capability
   table and the decoder chosen by name: H.264 on the GPU and AV1 through libdav1d in the same session,
   video only. Measure decode speed with the render loop running, with and without a frame-long lock. Force
   a hardware failure mid-file to test the fallback. Check the rotated and mirrored fixtures.
2. WASAPI audio and the clock, then seek, step, loop, mute and a device change.
3. Animations on the same item, Qt Multimedia removed. Run the dataset videos plus the edge cases: MKV,
   AVI, WMV, MPG, M4V, 3GP, a truncated file, 10-bit HEVC, HDR10, HLG, 5.1 audio.

**Option B:**

1. Way 2: an MSVC build loads libmpv and renders through D3D11 into the `QQuickRhiItem` on our device, with
   the external-content bracket and the debug layer on. H.264 on the GPU and AV1 in software, with
   `hwdec=d3d11va` and `hwdec-codecs` from our check. Resize while playing.
2. Seek, step and loop through the client API, with the QML transport. Measure opening time, the loop gap
   and the render-thread time per frame. Switch and unplug the audio device.
3. Way 1, built against our FFmpeg, with its runtime closure checked. Animations on the item, Qt
   Multimedia removed, the same files as A.

**Decisions before shipping either:** the app's own licence (it has none yet; way 1 links the GPL FFmpeg, so its libmpv is GPL whatever mpv's `gpl` option says), and for B whether way 1's
toolchain is acceptable (way 2 means two FFmpeg copies). The options are in "Research 2026-11-25 → Licences".

## Videos and animations (M2)

Our FFmpeg is BtbN's `gpl-shared` build, n9.0.2-39-g7adfbe40dc, at `D:\PhotoGallery\tools\ffmpeg-9.0`. It is the
only FFmpeg on this machine's tools path:
the app and the video test use one copy, and the test's x264 clips need the GPL encoders.

### Measured here

- **Dataset videos** (`pinterest`, 1,712 files at `D:\PhotoGallery\datasets\pinterest\Videos`): 837 H.264,
  748 VP9 and 127 VP8. There is no AV1, HEVC or PQ among them; 3 are HLG, and 523 are taller than 1080 px. Import: 1,712
  files with 0 failed, in 20.9 s (82 files/s, about 195 MB/s; `gallery import` on a scratch catalog, release build of
  2026-11-14). Thumbnails at 256 px: 1,712 with 0 failed, in 27.9 s (`gallery thumbs --height 256`, same run). 63 H.264 files carry a creation date.
- **What the dataset videos test** (header-only ffprobe of all 1,712, one at a time, with no failures):
  - All 8-bit 4:2:0 (5 of them full-range, `yuvj420p`); profiles: VP9 profile 0, H.264 Constrained Baseline (581), High (233) and Main (23).
  - 3 are HLG (ARIB-STD-B67 transfer; all `tumblr_*`), 8-bit. No PQ, no 10-bit. No Dolby Vision found, though one
    file has an ambient-viewing-environment side-data entry (not Dolby Vision).
  - No display-matrix rotation; 1,428 are stored portrait (height above width).
  - 1,080 have no audio. The other 632 are AAC: 625 stereo and 7 mono, mostly 44.1 kHz.
  - One has a variable frame rate; two video streams start more than 0.1 s late (an edit list or a delay).
  - Median length 6.3 s, longest 9.1 min.
  - So a player test bed also needs generated or sourced HEVC, AV1, 10-bit, PQ, Dolby Vision, rotated and
    mirrored, and surround files.
- **A display matrix's mirror comes after its rotation** (`av_display_matrix_flip` negates the first column
  for a horizontal flip, the second for a vertical one). EXIF mirrors before rotating, so a mirrored matrix's
  turn is reversed. `matrix_orientation` in `crates/video/src/lib.rs` handles that, and the unit test
  `display_matrix_orientations` covers the eight matrices ffmpeg writes.
- **Qt's player can't play AV1 on this GPU** (RTX 2060, Turing, which has no AV1 decoder):
  - Qt picks FFmpeg's built-in `av1` decoder with d3d11va. That decoder only decodes on the GPU. Qt checks
    only that `d3d11.dll` loads and creates a device per file, not that the GPU decodes the codec. FFmpeg then fails its
    hardware setup on every frame, and the decoder has no software path.
  - Qt falls back to software only when no hardware device or decoder is found, or the decoder fails to open
    (`qffmpegcodeccontext.cpp`, `qffmpeghwaccel.cpp`, Qt 6.12.0). AV1 fails per frame, after that point, so the
    result is 0 frames and no error shown.
  - The switch is `QT_FFMPEG_DECODING_HW_DEVICE_TYPES`, documented as a troubleshooting setting
    (`advanced-ffmpeg-configuration.qdoc`). It is global for every codec and read once per process. Other
    variables also change decoder behaviour (`QT_ENABLE_EXPERIMENTAL_CODECS`, `QT_FFMPEG_HW_ALLOW_PROFILE_MISMATCH`).
  - Our stopgap (`crates/desktop/cpp/gpu_device.cpp`, documented in `crates/desktop/src/gpu_device.rs`): ask D3D11 for the AV1 profile at startup and, without
    it, turn hardware decoding off. Then AV1 plays through libdav1d: about 60 frames in 2.4 s (25 fps) on a 3 s AV1 test clip, with our FFmpeg 9 DLLs beside
    `gallery.exe` (they contain libdav1d; the app's deploy copies them there, and `--no-ffmpeg` keeps Qt's out). With no copy of our FFmpeg DLLs beside the exe, Qt's own FFmpeg gives 0 frames for the same clip, even with the
    stopgap's value set. The loader takes the copy beside the exe first, whatever PATH says. Every codec decodes on the CPU on this machine.
  - Rust's `std::env::set_var` doesn't reach Qt here. Qt reads the C runtime's copy of the environment,
    which `SetEnvironmentVariableW` doesn't update, so the variable is set with `qputenv`.
- **FFmpeg can't loop animated WebP:** its `webp_anim` demuxer has no `read_seek`, and its own loop is off by
  default (`ignore_loop=1`). `ffmpeg -stream_loop 2` gives 10 frames instead of 30, while GIF and APNG give 30. Through Qt's player, an
  animated WebP plays once when looping is off. With infinite loops it stops after the first pass, because the
  demuxer does not loop itself by default and an infinite loop needs a seek back, which `webp_anim` does not
  support. Qt's run delivered 2 of 10 frames and logged the demuxer's skip warning at 24–43k lines a second. The
  CLI gives 8 frames in 3 s with `-stream_loop -1` and 30 with `-ignore_loop 0`, which loops inside the demuxer.
  The app does not send animated WebP to Qt's player: it plays through `AnimationPlayer`.
  - Qt's `AnimatedImage` has no APNG. Its WebP support comes from the `qwebp` image plugin, which the Qt 6.12
    kit ships; whether the app deploys it is a separate question.
  - So animations play through our own player (`crates/desktop/src/animation.rs`, on `pg_codec`'s decoders, which
    are built on the `image` crate). Frames are pushed into the `QVideoSink`. The player's loop is tested for GIF (`crates/render/src/player.rs`), and the codec test decodes WebP, GIF and
    APNG, checking the total duration and the first and last frame colours (`crates/library/tests/video.rs`). Per-frame durations and pause are not tested.
- **libwebp_anim merges identical frames:** 10 identical frames became one still image (lossy VP8 by default, VP8L with `-lossless 1`). Tests check the
  total duration, not frame counts.
- **The software scene graph draws no video frames.** Playback can only be checked through the
  sink's frame count and the player's position.

### Video player options (web research; player not chosen yet)

- **Qt Multimedia:**
  - A similar report is on the Qt Forum ( "Qt Multimedia 6.7.2 with ffmpeg cannot decode h264
    on Pi5", topic 159939; a reply says there are not enough checks that the chosen hardware decoder can do its job. It is a Pi 5 V4L2
    case, not the same codec path).
  - Qt's GStreamer backend page calls Qt Multimedia "not a general purpose streaming framework". That backend
    is the default on embedded Linux, not Windows.
- **libmpv:**
  - Its manual says that when hardware decoding isn't possible, "mpv will fall back on software decoding";
    it goes down the list of hardware decoders, then software, for each file.
  - It has `loop-file`, `frame-step` and HDR tone mapping.
  - Qt Quick integrations of libmpv use the OpenGL render API. KDE MpvQt expects Qt Quick on OpenGL
    (`QQuickWindow::setGraphicsApi(OpenGL)`) before its first window (it logs an error otherwise).
  - mpv PR #17764, "libmpv: add D3D11 render API backend", is merged (milestone 0.42.0, released 2026-11-05). mpv renders into an
    application-owned `ID3D11Texture2D` on the application's device. The latest release is 0.42.0 (2026-11-05).
  - Daily LGPL builds of libmpv from mpv's git exist (zhongfly/mpv-winbuild, `mpv-dev-lgpl-x86_64-…`; the
    2026-10-30 build). They link their own FFmpeg statically.
- **GStreamer:**
  - Its D3D11 decoders register only if the device passes a profile probe for that codec (AV1:
    `sys/d3d11/plugin.cpp:206`, `gstd3d11av1dec.cpp:321-324`). Where a decoder is absent, decodebin falls back to
    the software decoder by rank, so per-codec fallback is built in.
  - The Qt 6 sink in gst-plugins-good is `qml6glsink`, which takes GL textures (`qt6glrenderer.cc:589`). No D3D11
    Qt sink is `qml6glsink`, and it takes only GL memory (`ext/qt6/gstqml6glsink.cc:111`). GStreamer's GL upload
    has no D3D11 input, so a D3D11-decoded frame is copied to system memory and then uploaded to a GL texture. It is
    not zero-copy (source reading, 1.24 and main).
  - Costs: a third media stack and a large runtime. The Qt plugin requires Qt 6 with no minimum minor version
    (`ext/qt6/meson.build:57`). Its one Qt-specific call, `QQuickRenderTarget::fromOpenGLTexture`, is in Qt 6.12.0.
    Only a build can confirm it compiles.
- **QtAVPlayer** (github.com/valbok/QtAVPlayer: MIT, 459 stars):
  - FFmpeg decoding, frames into `QVideoSink` (copy-free D3D11 frames are labelled experimental), sound through
    `QAudioSink`, accurate seeking, frame stepping.
  - Its README promises automatic software fallback, but the source (`src/QtAVPlayer/qavdemuxer.cpp:280-307`,
    `785-807`, at commit 6f8d83a) falls back only when no hardware device context can be created. It does not check
    the GPU's codec support, does not name an AV1 decoder, and does not switch to software after per-frame decode
    errors. So it would not rescue the AV1 case above (source reading only, not run).
- **Rust:**
  - No mature player library. A July 2026 DEV post (one author) lists decoding toolkits and bindings, not player crates, and
    ffmpeg-next is in "maintenance-only mode".
  - GUI crates wrap GStreamer (`iced_video_player`) or libmpv (`egui_sharkplayer`, through `libmpv2`), or
    use FFmpeg directly (`egui-video`).
- **Options (not decided):** third-party wrappers (QtAVPlayer, MpvQt, `libmpv2`) are out: the integration
  stays ours. The two candidates are our own engine on our FFmpeg 9, or libmpv on a pinned master commit
  with the D3D11 render API. Both are written up below, with spikes.
  Animations stay on our own decoders either way.

## Research (2026-11-25)

The open questions above, researched from sources: Qt 6.12.0 (`C:\Qt\6.12.0\Src`), FFmpeg `release/9.0`,
mpv master `04b1afc4`, libplacebo master, Chromium, VLC, Firefox's cubeb, SDL, Kodi, the Microsoft and NVIDIA
docs and the ITU texts. Nothing was built or measured in the app; where a number is an estimate, it says so.
The corrections are folded into the sections above; this is the detail behind them.

### Shaders (qsb)

- The kit's `bin\` has no qsb; Qt Creator's is 6.11.2. The 6.12.0 qsb is the addon
  `qt.qt6.6120.addons.qtshadertools.win64_msvc2022_64` (3.3 MB: `qsb.exe` + `Qt6ShaderTools.dll`), via
  MaintenanceTool, `aqt … -m qtshadertools` (in CI: `modules: qtmultimedia qtshadertools`), or the
  PySide6-Addons 6.12.0 wheel. Unpacked beside the kit's DLLs it reports `qsb 6.12.0`.
- Compatibility: `QSB_VERSION` is 9 in 6.11.2 and 6.12.0 (`qshader_p.h`). `QShader::fromSerialized` loads
  every version it knows (1–9) and rejects newer ones, so old files load in newer runtimes, not the reverse
  (`qsb --qsbversion` bakes for older ones). 6.12 only hardens the reader. An NV12 shader baked by both qsbs was
  byte-identical, and qsb 6.12's `-d` read the 6.11.2 file.
- The D3D11 backend (`qrhid3d11.cpp:4683-4750`) takes `{DxbcShader, 50}`, else `{HlslShader, 50}` compiled at run
  time by `D3DCompiler_47.dll` (windeployqt deploys it). Bake `--hlsl 50`. `-c` runs fxc from PATH (installed in
  the Windows SDK, not on PATH); without it qsb 6.12 logs a failure, exits 0 and stores source.
- `qwebp.dll` is deployed by `just deploy` locally (windeployqt dry run). CI installs only `qtmultimedia`, so its
  kit likely lacks Qt Image Formats (not checked on a runner).

### Shared device

- Multithread protection makes each immediate-context call atomic (and DXGI calls such as `Present`);
  `Enter`/`Leave` hold the same lock across a series. Media Foundation recommends exactly this setup for decoders.
- QRhi D3D11 records a frame and replays it in one burst at `endFrame` (`qrhid3d11.cpp:1528-1540`) or at
  `beginExternal`, relying only on bindings it set itself. The decoder thread's calls (video-context decode calls,
  `CopySubresourceRegion`, staging `Map`) don't touch those bindings, so per-call interleaving is safe. Any other
  thread setting pipeline state (shaders, SRVs, render targets, `VideoProcessorBlt`) is not.
- FFmpeg (`hwcontext_d3d11va.h:81-96`) takes recursive `lock`/`unlock` callbacks, or installs its own mutex. It
  holds the lock from `DecoderBeginFrame` to `DecoderEndFrame` (`dxva2.c:917-1088`, dropping it on `E_PENDING`
  sleeps), around `CreateVideoDecoder` and `av_hwframe_transfer_data`. The last has an early return without
  unlocking when the staging texture fails (`hwcontext_d3d11va.c` ~457): with a lock shared with the render thread
  that deadlocks Qt, so we never call it with a shared lock; our ring copy is our own single call.
- Precedent: mpv turns on protection and copies on the render thread with no lock (`hwdec_d3d11va.c:128-137`,
  `:240-268`); VLC uses one app mutex around its display's prepare and display, passed to FFmpeg, plus protection.
- Qt has no lock hook, but `afterRendering` (before `endFrame`) and `frameSwapped` bracket the replay and
  `Present`, so a frame lock is possible unpatched. The frame-latency wait is in `beginFrame`, outside it.
- Recommendation for the spike: protection on; one `AVHWDeviceContext` for every decoder (current and
  neighbours) with `lock`/`unlock` = `ID3D10Multithread::Enter`/`Leave`; pipeline state only on the render thread;
  slot reuse after `frameSwapped`; the frame lock as a switch to measure; the debug layer on.
- Device loss in Qt 6.12, confirmed: `handleDeviceLoss` tears down and `createRhi` re-imports a still-set device
  (`qsgrhisupport.cpp:1228-1234`) without checking `GetDeviceRemovedReason`, and swapchain creation fails again,
  hence the loop and our `sceneGraphInitialized` reset. Our item's renderer is recreated and sees the new
  `QRhi`'s `nativeHandles()`. Simplest recovery that keeps `VIDEO_SUPPORT` and protection: in
  `sceneGraphInvalidated` (render thread, before `ensureRhi`), if the old device is removed, make a new one of ours
  and `setGraphicsDevice` it.
- NVIDIA: no documented limit on concurrent decode sessions (NVDEC application note); the RTX 2060 has one
  NVDEC (about 771 fps of 1080p H.264). The decoder cap is ours.

### Audio clock

- Heard position: see "Clock" above. Per fill, publish (media time of frame `GetPosition`, its QPC time, playing)
  atomically; the render tick extrapolates to the predicted present time, clamped to the last frame written.
  Smooth over 0.5–1 s (USB and Bluetooth positions step), slew video toward audio, hard-resync past about 100 ms,
  and keep a per-endpoint user offset (Bluetooth, HDMI report no device delay). PTS segments start at each seek,
  loop or gap.
- Choose frames against the predicted scan-out time, not now: the compositor's delay (about 16–50 ms) otherwise
  makes audio lead, the direction viewers notice first.
- Tolerances: EBU R37 +40/−60 ms (audio early/late), ITU-R BT.1359 acceptable +90/−185 ms. Acceptance criterion on
  wired outputs: mean within +15/−45 ms, spread (p95−p5) at most one refresh plus 5 ms.
- Measuring: FFmpeg's `avsynctest` lavfi source (flash plus beep) and a full-screen flash-and-beep clip, filmed
  with a phone at 240 fps (calibrated on hand claps, 2.9 ms per metre), analysed with `signalstats` and
  `silencedetect`; validate the method with `avsynctest dl=2`. Loopback capture is no use (taken before the device).
- Device changes: `IMMNotificationClient` callbacks only set a flag (they must not block or release); filter
  `eRender` and one role; debounce about 250 ms without skipping an unchanged ID; also `OnDeviceStateChanged`,
  `PKEY_AudioEngine_DeviceFormat` and `OnSessionDisconnected`. Rebuild on `AUDCLNT_E_DEVICE_INVALIDATED` or
  `_RESOURCES_INVALIDATED` with backoff (mpv retries at 1/10/100/1000 ms), re-base the clock, and keep video on the
  steady clock until the new position moves.
- `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY` handles the sample rate; keep swresample for channel layout and float.
  Automatic stream routing (`ActivateAudioInterfaceAsync`) works on desktop but gives no device ID, no notice of
  latency changes, and still needs the rebuild path; no desktop player uses it.
- MMCSS: "Audio" (Microsoft's shared-mode sample, cubeb). "Pro Audio" (mpv, Chromium) only if the thread just copies
  and applies gain, with a lock-free ring.

### libmpv

- Loop: `handle_loop_file` (`playloop.c:924-962`) seeks with `MPSEEK_FLAG_NOFLUSH`, which keeps only audio-output
  buffers; decoders and the VO are reset, and the core waits in `vo_wait_frame` for the first new frame
  (`video.c:1358-1365`). Local files with `cache=auto` have no back buffer (`demux.c:2648-2665`). Issue #13924
  (1-frame loop stutter) was closed as solved in main without a named commit.
- Preloading: `prefetch-playlist` (`playloop.c:819-820`, `loadfile.c:1286-1331`) starts at EOF, next file only, no
  decode. Several cores with one render context each are supported and may render concurrently (`render.h:74`).
  AMF filters with libplacebo clashed on a shared context (PR #18688); kasper93 suggested separate devices there.
- `d3d11va-zero-copy` applies with the render API; caveats: wrong sampling at padded edges and driver bugs
  (`options.rst:6572-6582`).
- Opening time: mp4 and mkv skip `avformat_find_stream_info`; avi, wmv and mpeg don't
  (`demuxer-lavf-probe-info=nostreams` helps). An explicit `hwdec=d3d11va` avoids probing.

### HDR and colour

- BT.2408-7 Annex 5 EETF: E1 = (E′ − PQ(LB))/(PQ(LW) − PQ(LB)); minLum and maxLum normalised the same way;
  KS = 1.5·maxLum − 0.5; Hermite above KS; black lift E2 + b(1−E2)⁴; back out. Only per-channel RGB and maxRGB stay
  inside the target volume (Table A5.1).
- mpv gl_video: hard clip to source peak, PQ normalised by division (LB = 0), no black lift, maxRGB blended
  mostly toward per-channel (deliberately desaturating highlights). libplacebo: subtraction normalisation, a
  variable black-lift exponent, default curve "spline", per-cone in IPT LMS, perceptual gamut LUT.
- Gamut for v1: mpv's desaturate (`video_shaders.c:898-907`), about six lines: mix toward BT.709 luminance when a
  channel is negative, divide by the max when above 1. libplacebo's perceptual LUT (about 1k lines) is overkill.
- HLG: BT.2100-3's extended gamma `1.2·1.111^log2(Lw/1000)` (mpv, libplacebo). BT.2446-1 Method B is the
  documented alternative for HLG to SDR (reference white at 86 %). 8-bit HLG: same chain, chosen by the tag.
- Qt 6.12.0's bugs, all confirmed in `qtmultimedia/src/multimedia`, plus new ones:
  - target not normalised: a 1000-nit source peaks at 26.6 nits, 4000 at 60.5, 10000 at 100; its knee starts at
    2.3 nits (`qvideotexturehelper.cpp:687-688`, `hdrtonemapper.glsl:22-36`); the tone-map luma assumes limited
    range (`nv12_bt2020_pq.frag:41`);
  - HLG: fixed `pow(lum, 0.2)` and a tone map that is a no-op on SDR, so 75 % reference white lands near 20 % of
    peak; 8-bit HLG goes to the plain `nv12` shader;
  - software 10-bit (`yuv_triplanar_p10`) and non-HDR P010 get no transfer or primaries step, so BT.2020 SDR is
    shown unconverted;
  - full-range BT.601 (`qvideotexturehelper.cpp:536-541`): R uses 1.772, G uses BT.2020 terms, B uses 1.42, from a
    wrong comment (b = 0.578, d and e swapped, 477-480). Limited-range 601 is right.
- Untagged rules: mpv `w >= 1280 || h > 576` → 709 (`csputils.c:190-193`); Chromium `h < 720` → 601, VP9/AV1
  always 709; VLC always 709; Qt `h > 576`; our importer `h >= 720` (`crates/video/src/lib.rs:298-299`).
- Dolby Vision profile 5 in ours: scale codes by 2^−bl_bit_depth, reshape each component (up to 9 pivots,
  polynomial or MMR pieces), RPU `ycc_to_rgb`, PQ EOTF, LMS→BT.2020 × RPU `rgb_to_lms`, PQ inverse, then BT.2020 PQ
  with peak `source_max_pq`/4095. Profile 7 FEL/NLQ needs an enhancement-layer decode: out of scope.

Shader-ready chain (SDR BT.709 out, 1.0 = 203 nits):

```
1. Decode: limited Y=(D-16·2^(n-8))/(219·2^(n-8)), C=(D-128·2^(n-8))/(224·2^(n-8));
   full Y=D/(2^n-1), C=(D-2^(n-1))/(2^n-1). (Kr,Kb): 601 (.299,.114), 709 (.2126,.0722), 2020 (.2627,.0593).
   R=Y+2(1-Kr)Cr; B=Y+2(1-Kb)Cb; G=Y-(2Kb(1-Kb)/Kg)Cb-(2Kr(1-Kr)/Kg)Cr.
2. Linearise by the transfer tag: SDR with 709/601 primaries skips 2-6; else L=203·E^2.4.
   PQ: L=10000·(max(E^(1/m2)-c1,0)/(c2-c3·E^(1/m2)))^(1/m1). HLG: inverse OETF, then OOTF at Lw=1000.
3. Tone map (PQ, HLG): m=max(R,G,B); E1=PQ(min(m,Lw))/PQ(Lw); maxLum=PQ(203)/PQ(Lw); KS=1.5·maxLum-0.5;
   Hermite above KS; rgb *= PQinv(E2·PQ(Lw))/m; rgb /= 203.
4. BT.2020→709 matrix [1.6605 -.5876 -.0728; -.1246 1.1329 -.0083; -.0182 -.1006 1.1187].
5. Gamut: desaturate toward Y when min<0; divide by max when max>1.
6. Encode with inverse BT.1886 (1/2.4); write RGB10A2 or dither to 8-bit.
```

### Test media

All generated with our FFmpeg 9.0.2 and checked with ffprobe (about 45 MB, 20 s). Recipes were kept in the
session scratchpad, not the repo; they belong in a fixture generator when the spike starts. What matters for
writing it:

- Colour tags must be set on the frames: `scale=out_color_matrix=M:out_range=R,format=PIX,setparams=range=R:
  color_primaries=P:color_trc=T:colorspace=S` (output options are overridden; `setparams` calls 601-625
  `bt470bg`).
- HDR metadata from encoder parameters (x265 `master-display`/`max-cll`, SVT-AV1 `mastering-display`/
  `content-light`) lands only in the bitstream; one re-encode from that file carries it into the container
  (`mdcv`/`clli`, Matroska Colour), also for VP9.
- `-display_rotation`, `-display_hflip`, `-display_vflip` exist in 9.0, as input options before `-i`, with
  `-c copy`.
- Encoders available: libx265, libsvtav1 4.2, libaom, librav1e, libvpx-vp9 (profile 2), wmv2/wmav2, mpeg2video/mp2,
  h263 + libopencore_amrnb (3GP), aac/ac3/eac3/libopus. Fails: E-AC-3 7.1 (silently downmixed to 5.1).
- Edge cases made: truncated mp4 (moov at end: won't open; faststart: 149 of 300 frames), truncated MKV, VFR,
  delayed video start (`-itsoffset`), B-frames, `-f ipod` for M4V (a bare `.m4v` gives a raw elementary stream).
- Dolby Vision 8.1, 8.4 and AV1 profile 10: dovi_tool 2.3.4 (MIT, one portable exe for `D:\PhotoGallery\tools`):
  `generate` an RPU from a JSON config, `inject-rpu` into the Annex B HEVC, then re-encode with
  `-dolbyvision 1 -strict unofficial` (x265 also needs VBV settings). A plain remux drops the configuration record;
  remuxing `dvh1` mp4 to mkv fails, so encode straight to mkv.
- Profile 5 can't be synthesised. Samples: Jellyfin test videos (repo.jellyfin.org/test-videos, CC BY-SA 4.0;
  1080p P5 48 MB, also P8.1, P8.4, HDR10 and 10-bit SDR, no HLG) and Dolby's browser test kit (Dolby's terms).

### Licences

Facts, not legal advice; a legal review comes before the first public binary.

- The repo is public with no licence (all rights reserved by default). `deny.toml` allows only permissive crate
  licences. The ExifTool-port crates are "Artistic 1.0 or GPL-1+" (D14), which fits a GPL app directly.
- The pinned BtbN build is `--enable-gpl --enable-version3` (GPLv3): x264, x265, xvid, xavs2, davs2, vidstab,
  frei0r, rubberband, avisynth and the dvd libraries are GPL-only, plus FFmpeg's own GPL filters. Dynamic linking
  and run-time loading don't change that (GPL FAQ): the shipped bundle is GPLv3, our files may be any
  GPLv3-compatible licence. GPLv3 §6 source: FFmpeg at `7adfbe40dc`, about 80 static external libraries at the
  BtbN tag's pins, and BtbN's scripts; BtbN's `download.sh` (`dl-only`) fetches them all.
- `lgpl-shared` (LGPLv3) loses the GPL encoders and filters but keeps all native decoders, dav1d, svt-av1, aom,
  vpx and hardware paths. pg-video uses no avfilter filters; only `crates/library/tests/video.rs:95` uses libx264
  (`libopenh264` would replace it).
- Qt under LGPLv3: source or offer, replaceable DLLs, notices. `qsb`, moc, rcc and windeployqt are GPLv3 with the
  Qt GPL exception, so their output doesn't make the app GPL. Qt Quick 3D is GPL-only (unused). The kit's SBOMs
  (`sbom\*.spdx`) list every bundled component.
- Qt's kit has its own FFmpeg DLLs with the same names as ours; the deploy must write the notices from what it
  actually copies.
- libmpv: LGPL with `-Dgpl=false`, but the combination with GPL FFmpeg is GPLv3 (confirmed by mpv's Copyright
  file). Its closure's LGPL members (libplacebo, fribidi, glib, libintl, libiconv) need source and texts; the rest
  are MIT, ISC, Apache-2.0, FTL, zlib or libpng.
- Comparable apps (mpv.net, IINA, Shotcut, Kdenlive, digiKam) are all GPL with LGPL Qt and GPL FFmpeg or mpv.
- Recommendation (from the research, not decided): GPL-3.0-or-later and a root LICENSE; publish the FFmpeg source
  bundle as a release asset each release; keep the LGPL route cheap (openh264 for the H.264 fixture).
