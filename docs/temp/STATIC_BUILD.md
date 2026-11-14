# Static Qt and FFmpeg build

Qt 6.12.0 and FFmpeg 9.0 built as static libraries with MSVC and the dynamic C runtime (`/MD`), so that `gallery.exe`
links them in instead of loading DLLs. The shared build (the default) is unchanged.

Status as of 2026-11-13. Build order: libjpeg-turbo, then FFmpeg, then Qt, then the app.

| Part | Where | Status |
|---|---|---|
| libjpeg-turbo (JPEG 8.0 API) | `tools/build_jpeg_turbo.py` | Built and checked, 2026-11-09 |
| FFmpeg 9.0 (static) | `tools/build_static_ffmpeg.py` | Built and checked with the PowerShell recipe (run 1, then runs 7 and 8). The Python port has not been run. |
| Qt 6.12.0 (static) | No recipe in the repo | Built and checked (pilot). Its configure and build scripts were in the work folder, not in the repo. |
| App on static Qt | `tools/static-qt/*.patch` | Runs offscreen with QML loaded. **The patches do not apply to `main` (see below).** |

## Pins

| Input | Pin | Where it is set |
|---|---|---|
| FFmpeg source | commit `7adfbe40dcde598f93a6963d2f7b22cd75202ccc` (`n9.0.2-39-g7adfbe40dc`), release `9.0.2` | `build_static_ffmpeg.py` |
| libjpeg-turbo | `darktorres/rust-turbojpeg` commit `945d853b47a76f98fd97ac5ff43830997712caa2`, the same commit as `turbojpeg-sys` in the root `Cargo.toml` | `build_jpeg_turbo.py` |
| Qt source | `qt-everywhere-src-6.12.0.zip`, 604,643,023 bytes, SHA-256 `a72e89c33726d742e67edf494abf7b625eaffc756673221c9823322ecd1a32d1`, MD5 `64889aee0623311cce096be5f445c7d1` | Qt pilot notes. Qt publishes no SHA-256 file; the SHA-256 was computed here, and the MD5 matches Qt's `md5sums.txt`. |
| MSVC toolset | `14.44.35207` (`vcvars64 -vcvars_ver`) | both Python recipes |
| Windows SDK | `10.0.26100.0` | `build_static_ffmpeg.py` checks for `dxva.h` |
| MSYS2 packages | `make 4.4.1-3`, `diffutils 3.12-1` | `build_static_ffmpeg.py` installs them if missing |

MSYS2 is expected at `C:\tools\msys64`. Visual Studio Build Tools 18 (2026) is expected at its default path. The pilot
also used CMake 4.4.4, Ninja 1.13.2, NASM 3.02 and Python 3.12.10.

The pinned FFmpeg in `.deps/ffmpeg` (`just setup`, the BtbN `gpl-shared` build) is a MinGW build
(`--target-os=mingw32`, from `ffmpeg -buildconf`). It cannot be linked into the MSVC app, which is why the static
FFmpeg is rebuilt with MSVC. Its DLLs still work through `libloading`, since they use the plain C ABI.

## One libjpeg for the whole process

Qt's `qjpeg` is built against Qt's bundled libjpeg, which has the JPEG 8.0 API. `turbojpeg-sys` builds libjpeg-turbo
with the JPEG 6.2 API by default. Both export `jpeg_*`, so the static link resolves Qt's calls to one copy, and opening
an image aborts with `Wrong JPEG library version: library is 62, caller expects 80`. In the shared build each library
is in its own DLL, so this does not occur.

Setting `WITH_JPEG8=ON` in `turbojpeg-sys` fixed the version number but not the structs: `JPEG parameter struct mismatch:
library thinks size is 624, caller expects 656`. Qt's bundled libjpeg is the IJG code, not libjpeg-turbo, and the two
lay out their structs differently. Matching the version is not enough.

Rejected: dropping Qt's JPEG. The app hands `QImage::from_data` every thumbnail (JPEG) and every Yandex original, which
can be any format Qt reads. Moving JPEG to Rust would leave the other formats in Qt, so the process would keep two
decoders.

**The fix:** one libjpeg-turbo, built once from the pinned fork, with `WITH_JPEG8=ON` (the API Qt expects),
`WITH_CRT_DLL=ON` and static libraries.

`python tools/build_jpeg_turbo.py [--work <dir>]` (default work folder `PG_HOME\static\jpeg-turbo`) produces:

- `<work>\prefix`: headers and `lib`, for Qt's `-system-libjpeg`.
- `<work>\link\turbojpeg.lib`: a copy of `turbojpeg-static.lib` under the name `turbojpeg-sys` links.

Only `turbojpeg-static.lib` goes into the process. Linking `jpeg-static.lib` as well would bring a second copy of
`jpeg_*`. The script checks that `jconfig.h` says `JPEG_LIB_VERSION 80`, and sets `core.autocrlf` to false so the
headers are the same on every machine. Its headers match the earlier hand build byte for byte.

Verified 2026-11-09: Qt's JPEG plugin decodes two photos (1080×1920 and 1920×1080) through this library, and a
missing file reports "File not found". Two runs in the same folder give 94 of 94 objects identical, apart from the
COFF TimeDateStamp (see Reproducibility).

## FFmpeg

`python tools/build_static_ffmpeg.py [--work <dir>]` (default work folder `PG_HOME\static\ffmpeg`). It needs MSVC and
MSYS2 on the machine, and it writes `<work>\manifest.json` with the commit, the tool versions, the configure line, the
library sizes and SHA-256s, the CRT references and the link-check output.

Steps:

1. **Source.** Fetch the full 40-character commit hash, with three tries (the network drops connections). The remote
   refuses short hashes. The source is cleaned to the pin, and `RELEASE` is checked to be `9.0.2`.
2. **Configure, in-tree**, with the prefix `/ffbuild/prefix`.
3. **Build** with `make ARFLAGS='-nologo -Brepro'`, then **install** with `DESTDIR` to a staging folder, which is moved to
   `<work>\prefix`.
4. **CRT check.** No library may reference `LIBCMT` (the static runtime). `MSVCRT` references are counted and recorded.
5. **Link check.** `tools/static-ffmpeg/static-check.c` is compiled with `cl /MD` against the five libraries plus
   `user32.lib` and `bcrypt.lib`, and run. It must print `h264 decoder: yes`, `d3d11va hwdevice: yes`,
   `dxva2 hwdevice: yes` and `open missing file: -2`. `dumpbin /dependents` must show no non-system DLLs.

The configure line (the pilot's flags, with the reasons the recipe records):

- `--toolchain=msvc --enable-static --disable-shared`
- `--extra-cflags='-MD -Brepro'`: the app's dynamic CRT, and reproducible objects. The dashes are deliberate: MSYS2
  rewrites `/MD` into a path.
- `--disable-debug`: no `/Z7`, because its debug data embeds the source paths.
- `--disable-programs --disable-doc --disable-autodetect --disable-network --disable-avdevice --disable-avfilter
  --disable-everything`
- `--enable-protocol=file`; demuxers `mov,matroska,avi,mpegts,image2`; decoders `h264,hevc,vp9,mpeg4,mjpeg`;
  parsers `h264,hevc,vp9,mpeg4video,mjpeg`; `--enable-swscale --enable-swresample`; `--enable-d3d11va --enable-dxva2`.

The demuxer, decoder and parser lists are the pilot's guess. They have not been checked against what pg-video needs.

Choices that follow from the configure line:

- **No `--enable-gpl`.** Configure reports `License: LGPL version 2.1 or later`. The static FFmpeg is LGPL, unlike the
  GPL shared build the app loads by default.
- **No PNG decoder.** Configure dropped it because zlib is missing. pg-video does not need PNG, so zlib was not added.

Results (PowerShell recipe, run 1, 2026-11-08): all checks passed in 243 s. The library majors are 61 (avutil), 63
(avcodec, avformat), 10 (swscale) and 7 (swresample), the same as the shipped DLLs. The link needs `user32.lib`
(`GetDesktopWindow`, from the DXVA2 device code) and `bcrypt.lib` (`av_get_random_seed`). FFmpeg's `.pc` files list
neither, because their `Libs.private` is empty, so the recipe carries the list itself. The libraries reference
`MSVCRT` only. The H.264 decoder and the `d3d11va` and `dxva2` device types are found. AV1 is not enabled, as
intended.

## Qt

There is no Qt recipe in the repo. The pilot's `scripts\qt-configure.ps1` and `scripts\qt-build.ps1` were in the work
folder (`D:\PhotoGallery\static`), which is not on this machine as of 2026-11-13.

Configure (the pilot's line, with the libjpeg settings from the section above):

    configure.bat -static -release -opensource -confirm-license -nomake tests -nomake examples -no-openssl -schannel
      -ffmpeg-dir <ffmpeg work>\prefix -system-libjpeg -feature-system-jpeg
      -submodules qtbase,qtshadertools,qtdeclarative,qtmultimedia,qtimageformats,qtsvg

- `-ffmpeg-dir` is the current FFmpeg prefix. The pilot used an earlier run's prefix.
- `-no-openssl -schannel`: TLS is Schannel, as the deployed app's `tls\qschannelbackend.dll` is. The kit's OpenSSL is
  not used.
- `-system-libjpeg` points at the libjpeg prefix through `JPEG_INCLUDE_DIR` and `JPEG_LIBRARY_RELEASE`. Use forward
  slashes: a backslash in `JPEG_LIBRARY` reaches the generated CMake as a `set()` with an escape sequence, and fails.
- `-feature-system-jpeg` is needed. Qt's `system-jpeg` condition tests `JPEG_FOUND`, which Qt sets in its own scope, so
  the condition is false even though the library is found (version 80). `-feature-` forces the feature on.
- Qt's summary warns that static linking disables dynamically loaded plugins. Every plugin the app uses must be
  imported by name.

Build: `cmake --build . --parallel 12`, then `cmake --install .` to `<work>\qt\prefix`. The pilot took about 50 minutes
from configure to install (8,452 steps, none failed). `prefix\bin` has no DLLs, and `qmake -v` reports 6.12.0.

Plugins are static libraries: `platforms` (`qwindows`, `qoffscreen`), `imageformats` (`qwebp` among them), `multimedia`
(`ffmpegmediaplugin`, `windowsmediaplugin`), `tls` (`qschannelbackend`), `networkinformation`, `iconengines` (`qsvgicon`).

### Link check and playback

A small Qt program (`linkcheck\`, not in the repo) links against `qt\prefix` and imports the app's plugin set by name:
`qwindows`, `qoffscreen`, the image formats, `qschannelbackend`, the FFmpeg and Windows media backends, `qsvgicon`.

- It links and starts on the `windows` and `offscreen` platforms. `qwebp` reads WebP.
- The DLLs it needs are `VCRUNTIME140`, `MSVCP140` (the `/MD` runtime, as intended) and `icu.dll`, a Windows 10 system
  component. No Qt or FFmpeg DLL.
- Qt logs `Using Qt multimedia with FFmpeg version 7adfbe4`, the pinned commit, so the FFmpeg backend is the static
  build.
- Playback works: a 2 s H.264 clip (320×240, 25 fps, made with the pinned `ffmpeg.exe`) plays through `QMediaPlayer` on
  the FFmpeg backend. It reports 51 frames, 2000 ms, then `EndOfMedia`. Decoding is in software.

The FFmpeg backend reports no decodable codecs or file formats (`QMediaFormat::supportedVideoCodecs(Decode)` and
`supportedFileFormats(Decode)` are empty). Qt's `qffmpegmediaformatinfo.cpp` takes the encoders as the decoders, and this
build has no encoders. Playback is unaffected. The app does not read these lists (no references in `crates\desktop`), but
any code that asks Qt what it can play would get nothing.

## App build against the static Qt

Two patches hold the source changes, so that the normal build does not carry them:

- `tools/static-qt/app-build.patch`: `crates/desktop/build.rs`, `crates/desktop/src/gui_app.rs`, and the new
  `crates/desktop/cpp/static_qt.cpp`.
- `tools/static-qt/pg-video.patch`: `crates/video/build.rs` and `crates/video/src/ffi.rs`. With `PG_FFMPEG_STATIC` set, the
  build links the FFmpeg archives and skips the DLL copy, and the function table points at the linked functions.

**The patches do not apply to `main` as of 2026-11-13.** `git apply --check` fails at `crates/desktop/src/gui_app.rs:18`,
`crates/video/build.rs:69` and `crates/video/src/ffi.rs:25`. They were made against an older tree and need rebasing before
use.

To apply them (once they are rebased):

    git apply tools/static-qt/app-build.patch
    git apply tools/static-qt/pg-video.patch

To undo them, apply each with `-R` in reverse order, or run `git checkout -- crates/desktop crates/video` when nothing
else in those folders is changed.

Build, with a separate target folder:

    cargo build -p pg-gallery --bin gallery      # debug, as verified

with these environment variables:

- `PG_QT_STATIC=1`: only this build sets it. The shared build's `build.rs` path is unchanged.
- `QT_DIR`: the static Qt prefix.
- `PG_FFMPEG_STATIC`: the static FFmpeg's `lib` folder.
- `FFMPEG_DIR`: the static FFmpeg prefix, for its headers.
- `TURBOJPEG_SOURCE=explicit`, `TURBOJPEG_STATIC=1`, `TURBOJPEG_LIB_DIR=<jpeg work>\link`,
  `TURBOJPEG_INCLUDE_DIR=<jpeg work>\prefix\include`.
- `CARGO_TARGET_DIR`: a separate folder.

Problems found by running the binary offscreen, in the order they were hit, with the fixes in the patch:

- **Link arguments do not propagate.** `cargo::rustc-link-arg` from `pg-desktop`'s build script does not reach the binary.
  The static libraries go out as `rustc-link-search` and `rustc-link-lib=static=<name>`.
- **Plugin symbol names.** Each plugin exports `qt_static_plugin_<name>`. The name drops the `Plugin` suffix for the TLS
  backends (`QSchannelBackend`). `dumpbin /symbols` on each plugin library lists the names.
- **Missing QML plugin `QtQml.WorkerScript`.** `QtQml`'s `qmldir` uses `import` lines, not `depends`, so a walk over
  `depends` alone misses it.
- **QML plugins need more modules.** `Qt6QuickControls2Impl`, `…Basic`, `…BasicStyleImpl`, `…Fusion`,
  `…FusionStyleImpl` and `Qt6MultimediaQuick`. Their unresolved symbols showed them. qt-build-utils reads only the
  modules' `.prl` files, not the plugins'.
- **Resources are not registered by themselves.** A static build runs no `qInitResources_*`, so the Controls styles'
  `.qml` files are missing until `Q_INIT_RESOURCE` runs. `cpp/static_qt.cpp` does this from `pg_static_qt_link`, which
  `gui_app::new` calls. The initializers have C++ linkage, so they sit in their own function: an `extern "C"` declaration
  gets an unmangled name and does not link.
- **Two libjpeg copies.** Fixed by the single libjpeg-turbo above.

Verified: `gallery.exe` (debug) starts offscreen, the QML loads, and it runs for 90 s with no JPEG error.

Not recorded: a release build, a run on the `windows` platform, and the app's test suite on the static build.

## Reproducibility

The standard in use: **same folder, same recipe, same object contents apart from the COFF TimeDateStamp** (bytes 4–7).

- FFmpeg, `recipe-run7` rebuilt in place: 348 of 413 objects identical. The other 65 differ only in the TimeDateStamp.
  The cause of the timestamp difference is not known.
- libjpeg-turbo: 94 of 94 identical, apart from the TimeDateStamp.
- The archives are not byte-identical (their member headers carry dates), so the comparison is on the extracted objects.

Across folders, the bytes differ and the standard does not apply. The causes that were removed:

- `__FILE__` in the libraries: the build is in-tree, so the source paths are relative.
- `FFMPEG_CONFIGURATION` (in `avutil`): the prefix is the fixed `/ffbuild/prefix`, installed through `DESTDIR`.
- `/Z7` debug data: `--disable-debug`.
- The COFF timestamp, which `/Brepro` makes a hash of the object's bytes: `-Brepro` on `cl` and `ARFLAGS` on `lib`.

The cause that remains: `cl` writes the absolute path of its output (`/Fo`) into each object, in the `LTLIB:OLDNAMES` record,
even without `/Z7`. `lib.exe` copies it into the archive, and so does `llvm-lib`. `cl /d1trimpath` is not supported by this
compiler (`C1007`). So an object's bytes depend on the folder it was built in, and the work folder is part of the recipe. A
fixed `subst` drive would make the path the same; that is not done.

Qt is not checked for reproducibility.

## Licence

- The static FFmpeg from this recipe is LGPL 2.1 or later (no GPL options, see above). It is not the GPL build the
  Justfile fetches.
- The static Qt is LGPL 3.0 or the Qt commercial licence. Which route a distributed build takes is an open decision. See
  `docs/temp/WEB_CLIENT_DESIGN.md`.

## Pitfalls

Each of these was hit while building, and the recipes now handle it.

- **MSYS2 rewrites flags.** `/MD` and `/Brepro` become paths. Use `-MD` and `-Brepro`.
- **MSYS2's `link` shadows MSVC's.** MSYS2's coreutils `link` is found first unless MSVC's `bin` comes before `usr\bin` on
  `PATH`.
- **No `cmp` in MSYS2's base.** Configure prints `cmp: command not found` on every comparison and still exits 0. Install
  `diffutils`, and read the configure log: a zero exit does not prove the configure ran.
- **Generated `.sh` files need LF line endings.** A CRLF script fails in bash. The recipe writes LF.
- **PowerShell reads `"$Toolset:"` as a drive-qualified name.** Write `${Toolset}:` (PowerShell recipe only).
- **Git converts line endings.** Git for Windows defaults to `core.autocrlf=true`, which put CRLF into the JPEG headers.
  The recipe sets it to false.
- **`vswhere.exe is not recognized`** is printed by `vcvars64.bat` itself. It is harmless.

## Open items

1. **Patches do not apply to `main`.** Rebase `tools/static-qt/app-build.patch` and `pg-video.patch`, and check the
   result by building.
2. **Qt's configure and build scripts are not in the repo.** Commit them (for example under `tools/static-qt/`), so the Qt
   build is reproducible like the FFmpeg and libjpeg builds.
3. **The Python FFmpeg recipe has not been run.** Run it from an empty folder. The last recorded runs are the PowerShell
   recipe's.
4. **The PowerShell recipes are still in `tools/`** (`build-static-ffmpeg.ps1`, `build-jpeg-turbo.ps1`), next to the Python
   ones that replaced them. `tools/README.md` describes them as wrappers, but they are full copies. Remove them or make
   them wrappers.
5. **Codec lists are unverified.** The demuxer, decoder and parser lists are the pilot's guess. Check them against what
   pg-video needs.
6. **Not checked:** the `networkinformation` plugin (`qnetworklistmanager`) is in the app but not in the link check's
   list. Add it if the app uses `QNetworkInformation`.
7. **Not recorded:** a release build and a `windows` platform run of `gallery.exe`.
8. **Reproducibility across folders** is not met, and the same-folder standard is not met for the timestamp.
   Qt is unchecked.
