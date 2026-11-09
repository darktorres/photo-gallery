# Static Qt and FFmpeg: build notes

Working notes for the static build (Qt 6.12 and FFmpeg 9.0, MSVC), kept as the build goes. When the recipe
(`tools/`) works from an empty folder, these notes explain why each step is there.

Work folder: `D:\PhotoGallery\static` (outside the repository: sources, builds and prefixes).

## FFmpeg recipe

`tools/build-static-ffmpeg.ps1` is the recipe: the pilot's steps, with the pins, the checks and the manifest.
`tools/static-ffmpeg/static-check.c` is its link check. A run writes `<Work>\manifest.json` (commit, tool
versions, configure line, library sizes and SHA-256, CRT references, link-check output).

Changes from the pilot, found while writing the recipe:

- **Fetch retries.** The first run's `git fetch` failed with `Connection was reset` (the network, not the source).
  The fetch now retries three times.
- **PowerShell parse error.** `"$Toolset:"` is read as a drive-qualified variable. Written as `${Toolset}:`.
- **Generated `.sh` files need LF line endings.** A PowerShell here-string in a CRLF `.ps1` gives CRLF, which bash
  rejects. The script writes with LF.

Run 1 (`D:\PhotoGallery\static\recipe-run1`, from an empty folder): passed all checks in 243 s. The link check
imports no non-system DLLs. Library versions match the pilot's.

### Reproducibility: what makes two runs differ

Run 2 (`recipe-run2`, from empty, same recipe) gave the same tool versions and the same member lists, and the same
library sizes, but different SHA-256s for four of the five libraries. `avcodec.lib` also changed size. Cause, checked by
comparing the extracted objects:

- **Object files:** 105 of 113 members differ. The differences are the COFF timestamp (bytes 4–7) and a few
  scattered bytes, which are the per-compile debug data that `/Z7` writes.
- **Archives:** the member headers carry dates, so the archive bytes differ even when the member list is the same.

Tests (`cl` and `lib` run twice, a few seconds apart):

- `cl /Z7` alone: objects differ between compiles.
- `cl /Z7 /Brepro`, same output name: identical. (The first test had different output names, which by itself
  changes the bytes, so it said nothing about `/Brepro`.)
- `lib /Brepro`: identical archives.

The recipe passes `-Brepro` to `cl` through `--extra-cflags='-MD -Brepro'`, and `ARFLAGS='-nologo -Brepro'` to `make`
so FFmpeg's `lib.exe` calls use it too. Dashes, not slashes: MSYS2 rewrites `/Brepro` into a path, as it did with
`/MD`.

Runs 3 and 4 (the final flags above, in two folders `recipe-run3` and `recipe-run4`) still differed in every library.
Cause: the absolute source paths (`D:/PhotoGallery/static/recipe-run3/src/libavutil/avstring.c`, and so on) are in
each library. `/Z7` writes them into the debug data. The pinned FFmpeg build was configured with `--disable-debug`,
so the recipe now does the same, which stops FFmpeg from passing `/Z7`. Runs 5 and 6 (`--disable-debug`, in two folders) still differed. Libraries shrank a lot (`avcodec.lib` 30 MB to
10 MB), which is the debug data going. The paths were still there, from three sources:

- **`__FILE__`** in `libavutil/avassert.h` and the log macros. It holds the source path as the compiler got it, so an
  out-of-tree build puts `D:/PhotoGallery/static/recipe-runN/src/...` into every library, with `runN` differing.
- **`FFMPEG_CONFIGURATION`** (`config.h`, compiled into `avutil`) holds `--prefix=<work folder>/prefix`.
- **`/Brepro`** makes the COFF timestamp a hash of the object's bytes. So the timestamp difference is a result of the
  path difference, not a separate cause.

Fix, now in the recipe: in-tree build (configure and make in the source folder, so `__FILE__` is relative); the
prefix is the fixed `/ffbuild/prefix` (the pinned build's prefix too), installed to a staging folder with `DESTDIR`
and then moved to `<Work>\prefix`. The source folder is cleaned to the pinned commit before each build.

Status after runs 7 to 8 (details below): the recipe builds, links and passes every check from an empty folder, but
two builds still differ byte for byte, so the reproducibility goal is not met yet.

- **Same folder, two builds (`recipe-run7` rebuilt in place):** checked over all five libraries, 413 objects in total.
  348 are identical. The other 65 differ only in bytes 4–7 of the COFF header, the TimeDateStamp, and nowhere else.
  No other byte differs. Accepted as the reproducibility standard for now: same folder, same object contents apart
  from the COFF timestamp. The archive files themselves are not byte-identical (their member headers carry dates),
  so the check compares extracted objects, not the archives. The cause of the timestamp variance is not known.
- **Different folders:** the difference is the absolute paths described below, not timestamps. Not covered by the
  standard above.
- **Different folders:** every library differs, from the absolute paths described below.

Runs 7 and 8 (in-tree, two folders) still differed in every library. The path was still there, in a place none of
the three fixes reached:

- **The object files.** `cl` writes the absolute path of its output (`/Fo`) into each object, in its `LTLIB:OLDNAMES`
  record, even without `/Z7`: `D:\PhotoGallery\static\brepro-test\sub\t.o`, from a relative `/Fo:sub\t.o`.
  `cl /d1trimpath` isn't supported by this compiler (`C1007`).
- **`lib.exe`** copies the same absolute path into the archive. `llvm-lib` (installed with LLVM, 
  `C:\Program Files\LLVM\bin`) does too, because it copies the object's bytes.

So an object's bytes depend on the folder it was built in. No flag in this toolchain removes that. What's left:
the work folder is part of the recipe. The same recipe at the same path should give identical bytes (checked
below); a different folder gives different bytes. Fixing the path (a fixed `subst` drive, say) is a later option.

### 2026-11-08: the pinned FFmpeg is not an MSVC build

`D:\PhotoGallery\static-notes-buildconf.txt` (from `ffmpeg -buildconf` on `.deps/ffmpeg/bin/ffmpeg.exe`):

- Built with gcc 16.2 through crosstool-NG, `--target-os=mingw32`, `--cc=x86_64-w64-mingw32-gcc`: a MinGW
  cross-build, not MSVC.
- Shared (`--enable-shared --disable-static`), GPL (`--enable-gpl --enable-version3`), with about 90 external
  libraries enabled (x264, x265, dav1d, libvpx, libaom, and so on).
- Source version: `n9.0.2-39-g7adfbe40dc`, so the source is commit `7adfbe40dc` of `FFmpeg/FFmpeg`.

Consequence: a static FFmpeg linked into `gallery.exe` (MSVC) has to be built with MSVC (`--toolchain=msvc`).
The pinned MinGW build can't be linked statically into the app. Its DLLs still work through `libloading`, since
they are plain C ABI.

### Pilot steps and what they showed

1. **Source.** `git fetch --depth 1` of the full commit hash `7adfbe40dcde598f93a6963d2f7b22cd75202ccc` (resolved
   with `gh api`; the remote refuses short hashes). Release file says `9.0.2`. A commit pin is fixed by its hash, so
   no tarball hash is needed.
2. **Tools.** MSYS2 at `C:\tools\msys64` (not `C:\msys64`). MSYS2 had no `make`: `pacman -S make` gave `make 4.4.1-3`.
   `perl 5.42.3-3` and `gawk 5.4.1-1` were already there. Outside the build: VS Build Tools 18 (2026), toolsets
   `14.44.35207` and `14.51.36231`, Windows SDK `10.0.26100.0`, CMake 4.4.4, Ninja 1.13.2, NASM 3.02 (on PATH
   from `C:\Program Files\NASM`), Python 3.12.10.
3. **Environment.** `scripts\msvc-env.ps1` runs `vcvars64.bat -vcvars_ver=14.44.35207` and puts MSVC's bin ahead of
   MSYS2's `usr\bin`. Without that order, MSYS2's `link` (coreutils) is found before MSVC's `link.exe`.
   `vswhere.exe is not recognized` is printed by vcvars itself and is harmless.
4. **Configure: two failures, both fixed.**
   - `--extra-cflags=/MD` became `C:/tools/msys64/MD`: MSYS2 converts `/MD` into a path. Use `-MD`. The compiler
     accepts it as the same flag.
   - The D3D11VA check failed only because of that path bug (the probe's source file was wrong), not because the
     SDK headers are missing: `dxva.h` and `d3d11.h` are in `Windows Kits\10\Include\10.0.26100.0\um`.
5. **CRT.** FFmpeg's configure sets no runtime flag, so `cl.exe`'s default applies. Pinned to `-MD` explicitly,
   to match the app's dynamic runtime. To verify from the built objects with `link /dump /directives`, not from the
   configure line.

Configure takes several minutes (it runs compile tests); it's the first long step.

6. **`cmp` missing.** MSYS2's base has no `cmp`, so configure printed `cmp: command not found` on every comparison
   and exited 0 anyway. A clean configure needs `diffutils` (`pacman -S diffutils`, 3.12-1). A zero exit doesn't prove
   the configure ran correctly: check the log for errors such as this one.
7. **PNG decoder dropped.** Configure disabled `png_decoder` because `inflate_wrapper` (zlib) is missing. Removed
   `png` from the decoder list rather than add zlib, since pg-video doesn't need it.
8. **Licence changes.** Without `--enable-gpl` and the GPL libraries, configure reports `License: LGPL version 2.1
   or later`. The pinned build is GPL. A static build with no GPL libraries is LGPL, so the app's GPL note in
   `crates/README.md` would no longer apply to a static build of this configuration. Check that before relying on it.
9. **Pilot configure line** (`scripts\ffmpeg-configure.sh`): `--toolchain=msvc --enable-static --disable-shared`,
   `--extra-cflags=-MD`, `--disable-programs --disable-doc --disable-autodetect --disable-network
   --disable-avdevice --disable-avfilter --disable-everything`, the file protocol, the demuxers `mov,matroska,avi,
   mpegts,image2`, the decoders `h264,hevc,vp9,mpeg4,mjpeg`, the parsers `h264,hevc,vp9,mpeg4video,mjpeg`,
   `--enable-swscale --enable-swresample`, `--enable-d3d11va --enable-dxva2`.
   The decoder and demuxer lists are a pilot guess, not a checked list of what pg-video needs.

10. **Build and install.** `make -j12` and `make install` (`scripts\ffmpeg-build.sh`), about three minutes. Output:
    `avcodec.lib` 29 MB, `avformat.lib` 7 MB, `avutil.lib` 7 MB, `swscale.lib` 8 MB, `swresample.lib` 1 MB (archives,
    not linked sizes). The prefix has `.pc` files, but their `Libs.private` is empty, so the system libraries a
    static link needs aren't recorded anywhere in the prefix.

11. **Verification** (`test\static-check.c`, linked with MSVC `cl /MD`):
    - Link needed `user32.lib` (`GetDesktopWindow`, from the DXVA2 device code) and `bcrypt.lib` (`av_get_random_seed`).
      The recipe must carry this list itself, derived from the link, not from `.pc` files.
    - Runs: library versions `avutil 3998054`, `avcodec 4129126`, `avformat 4129126`, `swscale 655718`, `swresample
      459110`. Those are majors 61, 63, 63, 10 and 7 (`version >> 16`), the same as the DLLs we ship (`avutil-61`,
      `avcodec-63`, `avformat-63`, `swscale-10`, `swresample-7`).
    - The H.264 decoder is found; the AV1 decoder isn't (not enabled, as expected). The `d3d11va` and `dxva2` hardware
      device types are found.
    - Opening a missing file returns an error through the demuxer, as expected.
    - CRT: the `.lib` files reference `MSVCRT` (`/MD`) and never `LIBCMT` (a byte grep, not `dumpbin /directives`,
      so it's a check of references, not a full audit). The test exe imports only system DLLs and
      `VCRUNTIME140.dll`, as `/MD` should give.

## Qt pilot

### Source

- `qt-everywhere-src-6.12.0.zip`, 604,643,023 bytes, from `download.qt.io/official_releases/qt/6.12/6.12.0/single/`.
- MD5 `64889aee0623311cce096be5f445c7d1`, which matches Qt's own `md5sums.txt`. Qt publishes no SHA-256 file, so the
  pin is the SHA-256 computed here: `a72e89c33726d742e67edf494abf7b625eaffc756673221c9823322ecd1a32d1`.
- Extracted to `D:\PhotoGallery\static\qt\qt-everywhere-src-6.12.0` (short path; `LongPathsEnabled` is 1 on this machine).

### Configure (`scripts\qt-configure.ps1`, pilot)

`configure.bat -static -release -opensource -confirm-license -nomake tests -nomake examples -no-openssl -schannel
-ffmpeg-dir <recipe-run7 prefix> -submodules qtbase,qtshadertools,qtdeclarative,qtmultimedia,qtimageformats,qtsvg`

- Configured in 227 s + 19 s, no errors.
- `-no-openssl`: the TLS backend is Schannel, as the deployed app's `tls\qschannelbackend.dll` is. The kit has
  OpenSSL; the app doesn't use it.
- Schannel, WebP (`qwebp` as a static plugin, for the Yandex panel) and the FFmpeg backend are all `yes` in the summary.
- Qt's summary warns: static linking disables dynamically loaded plugins. Every plugin the app uses (the Windows
  platform, the image formats, the QtQuick styles, the multimedia backends) must be imported explicitly.
- The app's CMake/cxx-qt setup and the deployed folder's plugin list are what the import list has to match.

### Build and install (`scripts\qt-build.ps1`, pilot)

- `cmake --build . --parallel 12`: 8,452 steps, no failed step, about 50 minutes configure-to-install.
- `cmake --install .` to `D:\PhotoGallery\static\qt\prefix`.
- `prefix\bin` has no DLLs. `lib` has the static Qt libraries (`Qt6Core.lib` is 38.8 MB, a static archive).
- Plugins are static libraries: `platforms` (`qwindows`, `qoffscreen`), `imageformats` (`qwebp` among them),
  `multimedia` (`ffmpegmediaplugin`, `windowsmediaplugin`), `tls` (`qschannelbackend`), `networkinformation`,
  `iconengines` (`qsvgicon`).
- `prefix\bin\qmake.exe -v` runs and reports Qt 6.12.0.
- Not checked yet: that `ffmpegmediaplugin.lib` links against our static FFmpeg (the check is a link, not a grep), and
  that a program can register these plugins and start. Not checked either: whether a second build reproduces these
  libraries byte for byte (the same-folder standard from the FFmpeg section has not been applied to Qt).

### Link check and playback (`linkcheck\`, not in the repository)

A small Qt program (`linkcheck\src\main.cpp`, `playback.cpp`) built with CMake against `qt\prefix`. It imports the
deployed app's plugin set by name (`qt_import_plugins`): `qwindows`, `qoffscreen`, the image formats, `qschannelbackend`,
the FFmpeg and Windows media backends, `qsvgicon`.

- **Links and starts**, `windows` and `offscreen` platforms. Qt's `qwebp` reads WebP.
- **DLLs:** none from Qt or FFmpeg. Non-system: `VCRUNTIME140`, `MSVCP140` (the `/MD` runtime, as intended) and `icu.dll`,
  a Windows 10 system component (present in `System32`).
- **FFmpeg backend in use:** Qt logs `Using Qt multimedia with FFmpeg version 7adfbe4`, the pinned commit.
- **Playback works:** a 2 s H.264 clip (320×240, 25 fps, made with the pinned `ffmpeg.exe`) plays through
  `QMediaPlayer` on the FFmpeg backend: 51 frames, 2000 ms, `EndOfMedia`. The Windows backend decodes it too (40 frames).
  Decoding is in software ("No HW decoder found").
- **Empty codec lists on the FFmpeg backend:** `QMediaFormat::supportedVideoCodecs(Decode)` and
  `supportedFileFormats(Decode)` return nothing. The cause is in Qt's source (`qffmpegmediaformatinfo.cpp`): for FFmpeg
  it takes "decoders = encoders", because "FFmpeg doesn't allow querying supported codecs for demuxers". Our build has
  no encoders, so Qt advertises no decoders, although playback works. The app doesn't read these lists (no references in
  `crates\desktop`), so this doesn't affect it. It would matter to any code that asks Qt what it can play.
- **Not imported yet:** the deployed `networkinformation\qnetworklistmanager` plugin (its CMake target name is not
  checked) is in the app but not in the check's list. Add it when the app's own plugin list is written, if the app
  uses `QNetworkInformation`.

## App build against the static Qt

**The source is not on `main`.** Normal builds don't need it, so it stays out of their source. It is kept as a patch,
`tools/static-qt/app-build.patch`, against the tree before it was taken out (`crates/desktop/build.rs`,
`crates/desktop/src/gui_app.rs`, and the new `crates/desktop/cpp/static_qt.cpp`). To use the static build:

    git apply tools/static-qt/app-build.patch
    git apply tools/static-qt/pg-video.patch

and the commands below. The second patch makes pg-video link the static FFmpeg instead of loading the DLLs: with
`PG_FFMPEG_STATIC` set, build.rs links the archives and skips the DLL copy, and ffi.rs points its function table at the
linked functions. The build also needs `FFMPEG_DIR` set to the static FFmpeg's prefix (its headers). Undo with `git apply -R` (or `git checkout -- crates/desktop`, when nothing else is changed there).

Build: `cargo build -p pg-gallery --bin gallery` with `PG_QT_STATIC=1`, `QT_DIR` = the static prefix,
`PG_FFMPEG_STATIC` = the static FFmpeg's lib folder, `TURBOJPEG_*` (see the libjpeg section), and a separate
`CARGO_TARGET_DIR`. Only that build sets `PG_QT_STATIC`; with the patch applied, the shared build's `build.rs` path is
unchanged.

What it took, in order (each a link or start failure, found by running the binary offscreen):

- **Link arguments do not propagate.** `cargo::rustc-link-arg` from `pg-desktop`'s build script does not reach the
  binary. The static libraries go out as `rustc-link-search` and `rustc-link-lib=static=<name>` instead.
- **Plugin class names.** A plugin's exported symbol is `qt_static_plugin_<name>`, where `<name>` drops the `Plugin`
  suffix for the TLS backends (`QSchannelBackend`, `QTlsBackendCertOnly`). `dumpbin /symbols` on each plugin library
  lists the names.
- **Missing QML plugin:** `QtQml.WorkerScript`. `QtQml`'s qmldir uses `import` lines, not `depends`, so a walk over
  `depends` alone misses it.
- **Modules the QML plugins are built on:** `Qt6QuickControls2Impl`, `…Basic`, `…BasicStyleImpl`, `…Fusion`,
  `…FusionStyleImpl` and `Qt6MultimediaQuick`. Their unresolved symbols showed them; qt-build-utils reads only the
  modules' `.prl` files, not the plugins'.
- **Resources are not registered by themselves.** A static build runs no `qInitResources_*`, so the Controls
  styles' `.qml` files (`ApplicationWindow.qml`, under `qrc:/qt-project.org/imports/QtQuick/Controls/Fusion/`) are
  missing until `Q_INIT_RESOURCE` runs. `cpp/static_qt.cpp` runs them from `pg_static_qt_link`, which
  `gui_app::new` calls. The initializers are C++-linkage, so they sit in a separate function (an `extern "C"`
  declaration gets an unmangled name and does not link).

With those, `gallery.exe` (debug) starts offscreen and the QML loads.

**Two libjpeg builds in one process (the problem).** Opening an image aborts: `Wrong JPEG library version: library is 62,
caller expects 80`. Qt's `qjpeg` is built against Qt's bundled libjpeg, which has the JPEG 8.0 API (`JPEG_LIB_VERSION
80`). `turbojpeg-sys` (the app's Rust turbojpeg, a git dependency) builds libjpeg-turbo with the default JPEG 6.2 API
(`WITH_JPEG8` off, `turbojpeg-sys`'s `CMakeLists.txt`). Both libraries export `jpeg_*`, and the static link resolves
Qt's calls to the 6.2 copy. In the shared build each is in its own DLL, so the clash does not occur.

Tried: `WITH_JPEG8=ON` in turbojpeg-sys (a local copy of the pinned fork, `D:\PhotoGallery\static\rust-turbojpeg`,
patched in `Cargo.toml` for the test, then reverted). The API version then matched, but the structs did not:
`JPEG parameter struct mismatch: library thinks size is 624, caller expects 656`, and the process crashes. Qt's
bundled libjpeg is the IJG code (`JPEG_LIB_VERSION 80`, with arithmetic coding and `BITS_IN_JSAMPLE` 8, 12 and 16
variants), not libjpeg-turbo, so the two never agree on the struct layout. Matching the version number is not enough.

Why not drop Qt's JPEG: the app hands `QImage::from_data` every thumbnail (JPEG, made by turbojpeg) and every Yandex
original, which can be any format Qt reads. Moving JPEG to Rust would still leave the other formats in Qt, so the
process would keep two decoders. One libjpeg for the whole process is the proper fix.

**The fix: one libjpeg-turbo, built once, used by Qt and the app.** `tools/build-jpeg-turbo.ps1` builds libjpeg-turbo
from the pinned fork's sources (the same commit as `turbojpeg-sys` in the root `Cargo.toml`), with `WITH_JPEG8=ON`
(API 8.0, as Qt expects), `WITH_CRT_DLL=ON` and static libraries, into `<Work>\prefix`. Then:

- **Qt:** `configure` with `-system-libjpeg -feature-system-jpeg` and `-DJPEG_INCLUDE_DIR` / `-DJPEG_LIBRARY_RELEASE`
  pointing at that prefix. Two details:
  - The path is passed with forward slashes. A backslash in `JPEG_LIBRARY` reaches the generated CMake as a
    `set()` with an escape sequence and fails with a syntax error.
  - `-feature-system-jpeg` is needed. Qt's `system-jpeg` condition tests `JPEG_FOUND`, which `qt_find_package` sets in
    its own scope, so the condition is false even though `WrapSystemJpeg` finds the library (`TRUE`, version 80).
    `-feature-` forces the feature on.
- **The app:** `turbojpeg-sys` in its `explicit` mode, with `TURBOJPEG_SOURCE=explicit`, `TURBOJPEG_STATIC=1`,
  `TURBOJPEG_LIB_DIR` = `<Work>\link` (a copy of `turbojpeg-static.lib` named `turbojpeg.lib`, the name it links) and
  `TURBOJPEG_INCLUDE_DIR` = `<Work>\prefix\include`. The static build only; the shared build is unchanged.
- **One archive:** only `turbojpeg-static.lib` goes into the process. Linking `jpeg-static.lib` as well would bring a
  second copy of the `jpeg_*` functions.

Verified (2026-11-09):

- **Qt's JPEG plugin** decodes real photos through the shared libjpeg-turbo: a small program (`D:\PhotoGallery\static\jpegcheck`)
  reads two JPEGs (1080×1920 and 1920×1080) and a missing file, which reports "File not found". The static
  `gallery.exe` (debug) on the same libjpeg runs offscreen for 90 s with no JPEG error.
- **Recipe:** `tools/build-jpeg-turbo.ps1` builds from an empty folder. Its headers match the hand build's byte for byte.
  Its first run had CRLF headers, because Git for Windows converts line endings by default. The recipe now sets
  `core.autocrlf` to false, so the bytes don't depend on the machine's Git settings.
- **Same folder, two runs:** 94 of 94 extracted objects are identical except bytes 4–7 of each COFF header, the
  TimeDateStamp. This meets the standard the FFmpeg recipe uses. Across folders the bytes differ, from the absolute
  source paths, as recorded for FFmpeg.
