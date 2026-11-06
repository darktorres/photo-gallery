# Video player: options (2026-10-30, not decided)

The viewer plays videos with Qt Multimedia today. That is a stopgap. FINDINGS, "Videos and animations (M2,
2026-10-30)", has the measurements and the web research behind this page. In short:

- On this RTX 2060, Qt's FFmpeg backend plays AV1 as 0 frames. Qt picks the decoder by device type and pixel
  format, with no check of the GPU, and opens FFmpeg's native `av1` decoder. That decoder can only use the
  GPU: FFmpeg's own profile check rejects the GPU (no AV1 decoding on Turing), and there is no software path
  to fall back to. `libdav1d` would have decoded it.
- The stopgap turns hardware decoding off for every codec whenever the GPU lacks AV1. It does this through
  the `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` environment variable, set from C++ because Qt reads the CRT
  environment.
- Animations already have their own player: `AnimationPlayer`, which uses image-crate decoders and pushes
  frames into a `QVideoSink`. FFmpeg's animated WebP demuxer can't seek back, so it can't loop.

Two replacements are on the table. Both keep the integration in our hands, with no third-party wrapper
libraries; they differ in who owns the playback core. Both were checked against the sources on 2026-10-30
(see "Validation"); this page includes the corrections.

## What the player has to do

- Local files only: the scanned containers (mp4, m4v, mov, webm, mkv, avi, wmv, mpg/mpeg, 3gp) with the
  codecs FFmpeg decodes.
- Decide per file between hardware and software decoding. Hardware when the GPU supports the codec,
  profile, size and bit depth; software (libdav1d for AV1) otherwise, or when hardware fails at run time.
- Autoplay with sound, loop without a gap, pause, exact seek, frame step, mute and volume.
  - No key or button steps backwards today (`Viewer.qml`). Step back costs a re-decode from the previous
    keyframe, so it is in v1 only if a UI needs it.
- The video's own orientation (the display matrix, as phones write it) plus the catalog turn, applied on
  display. Qt applies the display matrix today, so dropping Qt means doing it ourselves. Fitted, with no zoom
  (as in PLAN).
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
  - We create it at startup on one chosen adapter, with `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`, and turn on
    multithread protection with `ID3D10Multithread::SetMultithreadProtected`. Neither Qt nor FFmpeg turns
    that on for a device it is given (FFmpeg does only for devices it creates itself).
  - We hand it to Qt Quick with `QQuickWindow::setGraphicsApi(Direct3D11)` and
    `setGraphicsDevice(QQuickGraphicsDevice::fromDeviceAndContext(...))`, both public API. It must happen
    before the scene graph initialises (it has no effect afterwards), so the window is declared hidden and
    shown after the load, once it has the device. Check it worked through
    `rendererInterface()` (or the `qt.scenegraph.general` log line "Using existing native D3D11 device").
  - Qt borrows the device and context without taking a reference. Our owner object keeps them alive until
    the window and every decoder are gone.
  - No copies between devices, but one copy on the same device per decoded frame: the decoder reuses its
    texture slots, so each frame is copied out into a texture of ours.
  - **Qt's half is done** (`crates/desktop/cpp/gpu_device.cpp`, 2026-10-31): the device is made on the first
    adapter, the AV1 check runs on it, and the window, declared hidden, is handed it before it's shown.
    Multithread protection isn't on yet: nothing but Qt's render thread uses the device so far.
  - Device loss: with the device left set, Qt would keep retrying with the dead one. So the hand-over is
    forgotten once the scene graph is up (`sceneGraphInitialized`, on the render thread), and after a loss Qt
    makes a device of its own. The decoder will need the same: follow Qt's device, or get a new one of ours.
  - Qt's own FFmpeg backend works differently: a device of its own per file, and three copies per frame
    through a shared texture with a keyed mutex. That design is the fallback if one shared device stalls.
- **A `QQuickRhiItem` (public since Qt 6.7)** that draws the video into the scene.
  - Written in hand-written C++ (a header with `Q_OBJECT` and `QML_ELEMENT`, run through moc by
    `cxx-qt-build`'s `cpp_file`). Its renderer is not a QObject, so CXX-Qt can't subclass it.
  - Its hooks run on the render thread. `initialize()` runs on every sync, so it must be idempotent, and the
    colour texture can be recreated on resize, so its native handle is read every frame.
  - Raw D3D11 work in `render()` goes between `beginExternal()` and `endExternal()`. That runs Qt's recorded
    commands first and makes Qt rebind its state afterwards.
  - It also draws our animation frames, replacing `QVideoSink` + `VideoOutput`.
  - Offscreen (software scene graph) it draws nothing, so its tests need a real window.
- **Qt Multimedia goes away, in stages.** With it go Qt's FFmpeg plugin, the environment-variable stopgap
  (`choose_video_decoding` in `crates/desktop/cpp/gpu_device.*` and its call in `crates/desktop/src/lib.rs`), `windeployqt --no-ffmpeg`
  and `frame_sink.*`.
  Until the new player has sound, Qt Multimedia stays: it is today's only audio path.
- **The transport bar and keys stay in QML** and drive the item through properties and invokables. The seek
  bar sends a seek on every drag move, so the player coalesces seeks.
- **Licences:** the deploy step writes a licences folder (LGPL and GPL texts, a notice per bundled library,
  the FFmpeg configure line and a source offer) and the app gets an About entry.

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
    `ffmpeg-sys-next` does bind the D3D11 device and frames contexts.
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
- **Clock.** Audio is the master clock when there is audio. 1057 of the 1689 probed dataset videos have no
  audio, so the steady-clock path is the common case, not an edge case.
  - The heard position is the written media time minus the WASAPI padding and latency and swresample's
    delay. `IAudioClock` is only an average unless the endpoint reports a fixed frequency, so the A/V offset
    gets an acceptance criterion, measured with a microphone.
  - The clock starts when the first video frame and first audio buffer are ready (Qt starts it on the first
    packet, so its first frames come late and render back to back).
  - At each render tick the newest frame due is shown and older ones are dropped. Qt has no late-frame drop.
- **Hardware decoding:**
  - **Choose the decoder by name.** `av1` with the hardware device when the GPU decodes AV1, `libdav1d`
    otherwise; never by codec id, whose order isn't guaranteed.
  - **Before opening.** A table maps codec, profile, bit depth and chroma to the D3D11 decoder GUID and
    output format (NV12 or P010). It is checked once on the shared device with `CheckVideoDecoderFormat`,
    and per stream with `GetVideoDecoderConfigCount` for the frame size. VP8 is always software: this
    FFmpeg build has no VP8 D3D11 decoder.
  - **`get_format`** returns the hardware format only if the stream passed the table; it never panics across
    the FFI and handles being called again after a failed format. Set `AV_HWACCEL_FLAG_IGNORE_LEVEL`, never
    `ALLOW_PROFILE_MISMATCH`.
  - **At run time.** Fall back after 3 consecutive decode errors (mpv's rule), or when a watchdog sees no
    output frames. Errors are classified: `EAGAIN` and `EOF` are flow control, anything else counts. The
    fallback reopens in software, seeks to the keyframe before the current position and decodes up to it.
    mpv doesn't do that (it only replays packets before the first frame), so this part is ours.
  - **Pool.** `extra_hw_frames` covers the frames we hold; the number of open hardware decoders is capped.
- **Rendering.**
  - **Hardware frames:** at receive time, copy the decoder's slice (one `CopySubresourceRegion` moves both
    planes, as Qt's shipped code does) into a ring of 2–3 NV12 or P010 textures of ours, created once. A ring
    slot is reused only after the frame that sampled it has ended.
  - **Planes:** each texture is wrapped once per plane, R8 + RG8 (NV12) or R16 + RG16 (P010), as Qt does.
    QRhi has no NV12 format.
  - **Software frames:** upload the planes and use the same shaders. Animation frames upload as RGBA.
  - **Shaders:** there is no `qsb` in the Qt 6.12 kit. Qt Creator ships qsb 6.11.2, enough for the spike if
    its output loads in 6.12; otherwise we build qsb from `qtshadertools` and pin it. `build.rs` runs it and
    embeds the `.qsb` files.
- **Colour.** Each stage is its own step with a unit test:
  1. Matrix and range from the tags, with one rule for untagged video shared with the importer (Qt
     assumes BT.709 above 576 lines, our importer at 720 and up).
  2. BT.2020 to BT.709 primaries for every BT.2020 source, not only HDR.
  3. Transfer decode chosen by the transfer function, not the bit depth. The dataset has three 8-bit HLG
     files; Qt sends those through its plain SDR shader.
  4. Tone mapping, gamut clip, SDR encode.
- **HDR.** Our own BT.2390 tone mapping, specified from the standard, not ported from Qt.
  - Qt's version has bugs: its target luminance isn't normalised to the source's peak (an HDR10 source comes
    out far too dark), its HLG path uses a fixed gamma of 1.2 with no OOTF, and software-decoded 10-bit
    frames get no tone mapping at all. Its full-range BT.601 matrix also has wrong coefficients.
  - HLG gets the BT.2100 OOTF (gamma 1.2 + 0.42 log10(Lw/1000)). The source peak comes from the mastering
    metadata, then MaxCLL, then 1000 nits.
  - Dolby Vision is keyed on the stream's compatibility id. 8.4 (iPhone) shows as HLG. Profile 5 needs the
    IPT-PQ matrices, then reshaping; it is out of v1 unless we find samples. This FFmpeg build has the
    libplacebo filter (with `apply_dolbyvision`), a possible route that needs Vulkan.
- **Orientation.** The display matrix comes from `codecpar->coded_side_data` (FFmpeg 9 has no stream side
  data), composed with the catalog turn into one EXIF-style orientation, applied as a UV transform in the
  shader. The importer's version has a mirror bug to fix first: a horizontal flip comes out as EXIF 4
  (vertical) instead of 2.
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

- Roughly 6–8k lines of Rust plus the C++ item and its shaders. For scale, Qt's engine is about 5.1k lines
  of C++.
- **Main risks:**
  - One shared device: decoding and Qt's rendering on one immediate context. Multithread protection
    serialises single calls; the spike measures whether that suffices or a frame-long lock is needed (which
    can stall decoding for up to a vsync).
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
PR #17764, merged on 2026-10-15; the 0.42 milestone is open and the latest release is 0.41.0).

### B: pieces

- **Bindings.** Our own `bindgen` over `client.h`, `render.h` and `render_d3d11.h` (53 functions; the player
  needs about 20–25, through an allowlist). No `libmpv2` or other crates. Check `mpv_client_api_version()`
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
  - `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME` = 0, pacing from the frame's target time. Otherwise each frame
    blocks Qt's render thread for up to 50 ms.
  - mpv's update callback only posts a queued call to the GUI thread, which calls `update()`. Every
    `mpv_render_*` call stays on the render thread; control calls use the async API.
  - `mpv_render_context_report_swap` from `QQuickWindow::frameSwapped`.
- **Hardware decoding.** d3d11va decodes on our device from mpv's decoder thread, and mpv turns on
  multithread protection itself. By default that is one copy per frame on the same device; zero-copy is an
  option with documented driver caveats.
  - `hwdec=d3d11va` (not `auto-safe`, which also tries the copy and nvdec variants), with `hwdec-codecs` set
    per file from our AV1 check on the shared adapter.
  - mpv falls back to software after 3 decode errors. It has no check for a decoder that produces nothing,
    so a watchdog of ours reopens with `hwdec=no`.
- **Control.** Commands and properties go through the client API:
  - `loadfile`, `seek <t> absolute` (exact by default; `absolute+keyframes` while dragging), `frame-step`,
    `frame-back-step` (ignored while a seek is running), `loop-file=inf`, `pause`, `mute`, `volume`,
    `video-rotate` for the catalog turn (frames already come out upright by the display matrix).
  - Seeks and steps wait for `MPV_EVENT_FILE_LOADED`. Change events come back through
    `mpv_observe_property` and map to Qt properties.
- **Options:** `vo=libmpv`, `config=no` (no user config or scripts), `load-scripts=no`, `osd-level=0` and
  `osd-on-seek=no` (mpv draws its OSD into the video by default), `audio-display=no`, `keep-open=always`,
  `resume-playback=no`, `audio-device=auto`, `tone-mapping` (works on this renderer), and
  `gpu-shader-cache-dir` on D: (the default is `%LOCALAPPDATA%`).
  - `target-prim`, `target-trc` and `target-peak` are ours to set: the render API passes mpv no output
    colour space, and `target-colorspace-hint` needs gpu-next.
- **Audio.** mpv's WASAPI output reopens by itself on a default-device change, unplug or format change.
  If that reopen fails, the track is dropped and nothing retries it: we watch the track list, re-select the
  audio track when a device is back, and show a "no sound" state.
- **Building libmpv.** Two ways:
  1. **We build it** (the shipping route). Pinned mpv commit, meson with MSYS2 clang64 under D:, against our
     FFmpeg 9 DLLs (BtbN ships the MinGW `.dll.a` import libraries): one FFmpeg.
     - Dependencies beyond libplacebo and libass (freetype, fribidi, harfbuzz): shaderc and spirv-cross (the
       D3D11 renderer needs both at run time), Vulkan headers, lcms2. mpv also needs FFmpeg's avfilter and
       avdevice DLLs, which we don't deploy today.
     - Configure with `-Dgpl=false -Dlibmpv=true -Dcplayer=false`, every optional feature off, then enable
       d3d11 and its dependencies explicitly, so a missing one fails the configure instead of silently
       dropping the D3D11 renderer. Disable rubberband explicitly: it is GPL and `-Dgpl=false` doesn't
       exclude it.
     - The runtime is about 23 MSYS2 DLLs (about 26 MB: libc++, libplacebo, shaderc, spirv-cross, libass and
       its font stack…) plus `vulkan-1.dll`, which libplacebo imports. The deploy copies an explicit list,
       checked against the built DLL's imports.
     - Don't install MSYS2's mpv or ffmpeg packages: they are GPL builds, and their FFmpeg DLLs have the
       same names as ours.
  2. **A pinned daily LGPL build** (zhongfly/mpv-winbuild), for the spike only. Pin its sha256 and keep a
     copy on D:; the release tags are pruned and can be replaced. It lacks `render_d3d11.h` (take it from the
     same mpv commit), an MSVC `.lib` (generate one from the exports, or load the DLL at run time) and
     licence texts. It links its own FFmpeg statically (a second, different FFmpeg 9) and imports
     `vulkan-1.dll`.
- **Animations.** These stay on our own decoders and upload through our side of the item. mpv reads
  animated WebP through the same FFmpeg demuxer that can't seek back. Our other threads must not use the
  immediate context while mpv renders.

### B: size and risk

- Our side: the C++ item and bridge, the bindings, a QML adaptor, the audio and decoder watchdogs, and the
  build and deploy of libmpv (way 1). Line counts aren't established.
- **Main risks:**
  - The D3D11 render API is new and unreleased, self-merged with one review. The maintainer calls the render
    API "limited" and suggests window embedding for D3D11 instead. Device loss isn't reported on D3D11 (open
    PR #18700). Bugs in it are ours to work around until fixed upstream.
  - Way 1's toolchain and runtime: MSYS2 upkeep and a 23-DLL closure on every mpv bump.
  - Opening time and preloading. Playlist prefetch is off by default and only opens the next file's
    demuxer; a pre-decoded neighbour needs a second mpv core with its own render context.
  - The loop is an end-of-file seek back to the start; whether it's gapless is unmeasured.
  - Less control: mpv's behaviour comes through its options. Anything mpv doesn't expose, we can't change.

### What B gives over A

- **The most battle-tested playback core,** for less code of ours: sync, seeking, frame stepping, looping,
  per-file fallback and audio device changes.
- **The widest tolerance for odd files.**
- **Not** better HDR than A through this API. The render API runs mpv's older renderer (vo_gpu's
  `gl_video`): BT.2390 tone mapping, but not libplacebo's renderer, and Dolby Vision only as its base layer
  (no reshaping). gpu-next in the render API has been an open request since 2022 (mpv issue #10810).

## Comparison

| | A: own engine | B: libmpv |
|---|---|---|
| Our code | ~6–8k lines + C++ item and shaders | C++ item and bridge, bindings, watchdogs + mpv build |
| FFmpeg copies | one (ours) | one (way 1) or two (way 2) |
| Toolchain and runtime added | qsb (Creator's, or built once) | MSYS2 + meson, ~23 DLLs + vulkan-1 (way 1) |
| Hardware fallback | ours: table, decoder by name, 3 errors, watchdog | mpv's, plus our AV1 check and a watchdog |
| HDR | our BT.2390 + HLG OOTF; DV 8.4 as HLG, no profile 5 | gl_video BT.2390; DV as base layer |
| Audio device changes | ours | mpv's, plus a re-select when reopening fails |
| Odd files | depends on us | mature |
| Gallery-specific control | full | through mpv options |
| Neighbour preloading | built in | a second core per neighbour |
| Animations | same pipeline | our decoders, our upload |
| Upstream risk | FFmpeg API only | unreleased, "limited" render API |

## Validation (2026-10-30)

Each option was checked against the sources: Qt 6.12's source, FFmpeg 9's headers and DLLs, the Windows SDK,
mpv and libplacebo master, PR #17764, a prebuilt libmpv, the crates and the dataset (a header-only ffprobe of
all its videos). About 115 claims for A and 70 for B were checked against the cited lines. The corrections are
in the sections above; the ones that moved the comparison:

- **A:** the AV1 root cause (the decoder choice, not a missing check), thread safety on one shared device,
  the ring of copied frames, the missing orientation and audio-gain designs, no `qsb` in the kit, Qt's HDR
  bugs, and the size (6–8k lines, not 4–6k).
- **B:** the HDR advantage is gone (the render API doesn't use libplacebo's renderer), `gpu-next` isn't an
  option there, the render step needs the external-content bracket, way 1 needs shaderc, spirv-cross and a
  23-DLL runtime, and way 2 is a spike artifact only.
- **Both:** the device must be handed to Qt before the window is shown, Qt can't recover a lost imported
  device, the item is hand-written C++, and nothing ships licence texts yet.

## Spikes

Run the chosen option in this order, hardest part first, and stop at the first step that fails badly. The
first step of either doubles as the decision test.

**Option A:**

0. Bake one NV12 shader with Qt Creator's qsb 6.11.2 and load it in the 6.12 runtime.
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

**Decisions before shipping either:** the app's own licence (it has none yet), and for B whether way 1's
toolchain is acceptable (way 2 means two FFmpeg copies).
