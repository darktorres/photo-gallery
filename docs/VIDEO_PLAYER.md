# Video player: options (2026-10-30, not decided)

The viewer plays videos with Qt Multimedia today. That is a stopgap. FINDINGS, "Videos and animations (M2,
2026-10-30)", has the measurements and the web research behind this page. In short:

- Qt's FFmpeg backend picks the hardware decoder without checking that the GPU supports the codec profile.
  On this RTX 2060, AV1 therefore plays 0 frames.
- The stopgap turns hardware decoding off for every codec whenever the GPU lacks AV1. It does this through
  the `QT_FFMPEG_DECODING_HW_DEVICE_TYPES` environment variable, set from C++ because Qt reads the CRT
  environment.
- Animations already have their own player: `AnimationPlayer`, which uses image-crate decoders and pushes
  frames into a `QVideoSink`. FFmpeg's animated WebP demuxer can't seek back, so it can't loop.

Two replacements are on the table. Both keep the integration in our hands, with no third-party wrapper
libraries; they differ in who owns the playback core.

## What the player has to do

- Local files only: the scanned containers (mp4, m4v, mov, webm, mkv, avi, wmv, mpg/mpeg, 3gp) with the
  codecs FFmpeg decodes.
- Decide per file between hardware and software decoding. Hardware when the GPU supports the codec,
  profile, size and bit depth; software (dav1d for AV1) otherwise, or when hardware fails at run time.
- Autoplay with sound, loop without a gap, pause, exact seek, frame step, mute and volume.
- The catalog turn applied on display. Fitted, with no zoom (as in PLAN).
- Draw inside the Qt Quick scene, so QML controls can sit on top of the video. A native child window can't
  do that.
- Correct colours: BT.601/709/2020 matrices, limited and full range, 8- and 10-bit. HDR tone-mapped to
  SDR.
- Open fast when stepping through a folder. Ideally the neighbouring videos are opened ahead of time.
- LGPL-compatible. Nothing of ours on C:.

## Shared groundwork (needed by both)

- **One D3D11 device for Qt and the decoder.** We create the device ourselves at startup, with
  `D3D11_CREATE_DEVICE_VIDEO_SUPPORT` and multithread protection on. We hand it to Qt Quick with
  `QQuickWindow::setGraphicsDevice(QQuickGraphicsDevice::fromDeviceAndContext(...))`, which is public API.
  Decoded textures then never leave the GPU, and no copies are needed between devices.
  - Qt's own backend decodes on a device of its own and copies each frame to Qt's device through a shared
    texture.
- **A `QQuickRhiItem` (public since Qt 6.7)** that draws the video into the scene.
  - It also draws our animation frames, replacing `QVideoSink` + `VideoOutput`.
- **Qt Multimedia goes away.** With it go:
  - Qt's FFmpeg plugin;
  - the environment-variable stopgap (`crates/ui/cpp/video_decoding.*`);
  - `windeployqt --no-ffmpeg`;
  - `frame_sink.*`, which the new item replaces.
- **The transport bar and keys stay in QML** and drive the item through properties and invokables.

## Option A: our own playback engine on our FFmpeg 9

We write the player on top of the FFmpeg 9.0 we already pin (BtbN `lgpl-shared`, `FFMPEG_DIR`).

References, for design ideas only:

- Qt Multimedia's FFmpeg backend: the playback engine, demuxer, stream decoder, video and audio renderers,
  time controller, and its D3D11 hardware-acceleration code;
- ffplay.c, a complete FFmpeg player in about 4k lines of C;
- mpv's hardware-decoding fallback logic.

### A: pieces

- **Bindings.** A thin safe layer of our own over `ffmpeg-sys-next`, covering what the player uses.
  `ffmpeg-next` is in maintenance-only mode, so the player won't build on it. The importer
  (`crates/core/src/video.rs`) can move to the same layer later.
- **Threads and queues.** Demux → video decode and audio decode, with bounded packet and frame queues.
  Packet serial numbers drop stale data after a seek, as ffplay does.
- **Clock.** Audio is the master clock. Each video frame is shown when its timestamp comes due on the clock,
  or dropped if it is late. Files without audio use the system clock.
- **Hardware decoding:**
  - **Before opening.** Map the codec, profile and bit depth to the D3D11 decoder profile GUID. Check it with
    `ID3D11VideoDevice::CheckVideoDecoderFormat` and `GetVideoDecoderConfigCount` for the frame size.
    `video_decoding.cpp` already does this check for AV1.
  - **At run time.** If the first frames fail or come out corrupt, reopen in software at the same position.
- **Rendering.**
  - **Hardware frames:** copy the decoder's texture-array slice into the item's NV12 or P010 texture, then
    convert to RGB in our shader with the stream's matrix, range and transfer function.
  - **Software frames:** upload the planes (YUV) and use the same shader. Animation frames upload as RGBA.
  - The shaders are compiled with Qt's `qsb` at build time.
- **HDR.** A tone-mapping pass for PQ (HDR10) and HLG, BT.2390 style, to SDR.
  - iPhone Dolby Vision (profile 8.4) has an HLG base layer, so it shows correctly as HLG.
  - Profile 5 (no backward-compatible base layer) would show wrong colours. Handling it needs the Dolby
    Vision reshaping that libplacebo implements.
- **Audio.**
  - Output: WASAPI directly, which needs the `windows` crate for COM. It gives the exact clock (through
    `IAudioClock`) and notifications when the default device changes.
  - Format conversion: swresample.
- **Transport.**
  - Exact seek: seek to the keyframe before the target, then decode and discard up to the target.
  - Frame step forward: decode one frame. Frame step back: seek to the previous keyframe and decode up to the
    frame before the current one.
  - Gapless loop: the demuxer seeks to the start ahead of time, so the first frame is already decoded when
    the last one is shown.
- **Preloading.** Neighbouring videos are opened and decoded up to their first frame while the current one
  plays, and dropped when you move away.

### A: size and risk

- Roughly 4–6k lines of Rust plus about 500 lines of C++ and shader code.
- **Main risks:**
  - A/V sync on messy files: variable frame rate, broken or missing timestamps, MP4 edit lists. FFmpeg's
    demuxers handle most of this; the player must not get in the way.
  - The audio device changing or disappearing mid-play.
  - HDR quality relative to libplacebo, and Dolby Vision profile 5.
  - Bugs only show on files we haven't seen. The 1712 dataset videos and generated edge cases are the test
    bed.

### What A gives over B

- **One FFmpeg, ours,** used for import, thumbnails and playback.
- **Full control over gallery behaviour:**
  - preloading neighbours;
  - what stepping and looping mean;
  - instant first frame;
  - one render path for videos and animations.
- **No extra toolchain.**

## Option B: libmpv, integrated by us

We embed mpv's player core through libmpv. We pin an mpv master commit for the D3D11 render API (mpv
PR #17764, merged for 0.42.0, not yet in a release).

### B: pieces

- **Bindings.** Our own `bindgen` over `client.h` and `render.h`: about 40 functions, a stable C API. No
  `libmpv2` or other crates.
- **Render bridge.** The `QQuickRhiItem` creates an `mpv_render_context` with the D3D11 backend on our shared
  device. In the item's render step, mpv draws into the item's texture. mpv's update callback schedules a
  repaint.
  - Whether mpv's hardware decoding can run on the client's device has to be checked in the PR. If it
    can't, mpv decodes on a device of its own and copies, as Qt does.
- **Control.** Commands and properties go through the client API:
  - `loadfile`, `seek … exact`, `frame-step`, `frame-back-step`, `loop-file=inf`, `pause`, `mute`,
    `volume`, `video-rotate`.
  - Change events come back through `mpv_observe_property` and map to Qt properties.
- **Options:**
  - `hwdec=auto-safe`: mpv tries each hardware decoder, then software, per file.
  - `vo=libmpv`, `audio-display=no`, `keep-open`, `gpu-next`, and `tone-mapping` for HDR.
  - `config=no`, so a user's mpv.conf doesn't apply.
- **Building libmpv.** Two ways:
  1. **We build it.** Pinned mpv commit, meson with MSYS2 clang64 in `D:\PhotoGallery\tools\msys64`,
     `-Dgpl=false`, against our FFmpeg 9 DLLs: one FFmpeg. It also needs libplacebo and libass (with
     freetype, fribidi, harfbuzz), built from pinned sources or meson wraps.
     - Not yet checked: whether BtbN's package has the MinGW import libraries (`.dll.a`) or they must be
       generated.
  2. **A pinned daily LGPL build** (zhongfly/mpv-winbuild, `mpv-dev-lgpl-x86_64-<date>`). No toolchain, but
     it links its own FFmpeg statically, so we ship two FFmpeg copies.
- **Animations.** These stay on our own decoders and upload through our side of the item. mpv reads
  animated WebP through the same FFmpeg demuxer that can't seek back.

### B: size and risk

- Roughly 1–2k lines of Rust and C++, plus the mpv build (way 1) or a pinned download (way 2).
- **Main risks:**
  - The D3D11 render API is new and unreleased. Bugs in it are ours to work around until fixed upstream.
  - Building mpv and its dependencies on Windows: MinGW toolchain upkeep (way 1).
  - Opening time. mpv's per-file setup has to be measured. Preloading neighbours means more mpv instances,
    or mpv's playlist prefetch.
  - Less control: mpv's behaviour comes through its options. Anything mpv doesn't expose, we can't change.
  - A large runtime: libmpv plus libplacebo and libass, and in way 2 a second FFmpeg.

### What B gives over A

- **The most battle-tested playback core,** for less code of ours: sync, seeking, frame stepping, looping
  and per-file fallback.
- **The best HDR and Dolby Vision handling** (libplacebo), and the widest tolerance for odd files.
- **Upstream keeps fixing it.** mpv is actively maintained.

## Comparison

| | A: own engine | B: libmpv |
|---|---|---|
| Our code | ~4–6k lines | ~1–2k lines + mpv build |
| FFmpeg copies | one (ours) | one (way 1) or two (way 2) |
| Toolchain added | none | MSYS2 + meson (way 1) |
| Hardware fallback | ours: capability check + run-time fallback | mpv's `hwdec=auto-safe` |
| HDR | our tone mapping (PQ, HLG); no DV profile 5 | libplacebo, DV included |
| Odd files | depends on us | mature |
| Gallery-specific control | full | through mpv options |
| Neighbour preloading | built in | instances or playlist prefetch |
| Animations | same pipeline | our decoders, our upload |
| Upstream risk | FFmpeg API only | unreleased render API |

## Spikes

Run the chosen option in this order, hardest part first, and stop at the first step that fails badly.

**Option A:**

1. The shared device, the `QQuickRhiItem` and hardware decoding with the capability check and software
   fallback. Prove it with H.264 on the GPU and AV1 through dav1d in the same session, video only.
2. WASAPI audio and the audio clock, then seek, step and loop.
3. Animations on the same item, Qt Multimedia removed. Run the 1712 dataset videos plus the audit's edge
   cases: MKV, AVI, WMV, MPG, M4V, 3GP, a truncated file, 10-bit HEVC.

**Option B:**

1. libmpv (way 2 first, for speed) rendering through D3D11 into the `QQuickRhiItem` on the shared device.
   H.264 on the GPU and AV1 through dav1d in the same session.
2. Seek, step and loop through the client API, with the QML transport. Measure opening time.
3. Way 1, building against our FFmpeg. Animations on the item, Qt Multimedia removed, the same dataset and
   edge cases as A.

Whichever spike is run, the step 1 checks double as the decision test: if A's step 1 turns into a
sinkhole, switch to B, and the other way round.
