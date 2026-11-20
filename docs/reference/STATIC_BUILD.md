# Static Qt and FFmpeg build

Qt 6.12.0 and FFmpeg 9.0 built as static libraries with MSVC and the dynamic C runtime (`/MD`), so that `gallery.exe`
links them in instead of loading DLLs. The shared build (the default) is unchanged.

Status as of 2026-11-14. Build order: libjpeg-turbo, then FFmpeg, then Qt, then the app.

| Part | Where | Status |
|---|---|---|
| libjpeg-turbo (JPEG 8.0 API) | `tools/build_jpeg_turbo.py` | Built and checked 2026-11-09 (by hand). The Python recipe ran from an empty folder on 2026-11-13 and wrote its manifest. |
| FFmpeg 9.0 (static) | `tools/build_static_ffmpeg.py` | The Python recipe ran from an empty folder on 2026-11-13: every check passes. Earlier, the PowerShell recipe (runs 1, 7 and 8). |
| Qt 6.12.0 (static) | `tools/build_static_qt.py` | Configure verified 2026-11-13: Schannel, system libjpeg, WebP and FFmpeg are all on. Its build and install steps are not run by the recipe yet (see Qt). |
| App on static Qt | `tools/static-qt/*.patch` | Rebased onto `main` on 2026-11-13. The shared build checks. The static configuration compiles. The static link is not run yet (see Open items). |

## Pins

| Input | Pin | Where it is set |
|---|---|---|
| FFmpeg source | commit `7adfbe40dcde598f93a6963d2f7b22cd75202ccc` (`n9.0.2-39-g7adfbe40dc`), release `9.0.2` | `build_static_ffmpeg.py` |
| libjpeg-turbo | `darktorres/rust-turbojpeg` commit `28b8b86af795d5e6f1402c978e62facd61952da2`, the same commit as `turbojpeg-sys` in the root `Cargo.toml` | `build_jpeg_turbo.py` |
| Qt source | `qt-everywhere-src-6.12.0.zip`, 604,643,023 bytes, SHA-256 `a72e89c33726d742e67edf494abf7b625eaffc756673221c9823322ecd1a32d1`, MD5 `64889aee0623311cce096be5f445c7d1` | `build_static_qt.py`. Qt publishes no SHA-256 file; the SHA-256 was computed from the zip, and its MD5 matches Qt's `md5sums.txt`. |
| MSVC toolset | `14.44.35207` (`vcvars64 -vcvars_ver`) | all three Python recipes |
| Windows SDK | `10.0.26100.0` | `build_static_ffmpeg.py` checks for `dxva.h` |
| MSYS2 packages | `make 4.4.1-3`, `diffutils 3.12-1` | `build_static_ffmpeg.py` installs them if missing |

MSYS2 is expected at `C:\tools\msys64`. Visual Studio Build Tools 18 (2026) is expected at its default path. The pilot
also used CMake 4.4.4, Ninja 1.13.2, NASM 3.02 and Python 3.12.10.

The pinned FFmpeg in `.deps/ffmpeg` (`just setup`, the BtbN `gpl-shared` build) is a MinGW build
(`--target-os=mingw32`, from `ffmpeg -buildconf`). Its pin is a month-end BtbN build, at the commit above; `just lint`
checks both (`tools/check_ffmpeg_pin.py`), and the build folder records the pin it was fetched at (`.pin`). It cannot be linked into the MSVC app, which is why the static
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

Only `turbojpeg-static.lib` goes into the process. It carries the `jpeg_*` functions as well as the TurboJPEG API
(checked on the 2026-11-13 build: `jpeg_CreateDecompress` and `tjInitDecompress` are both in it), so Qt is given that same
archive and not `jpeg-static.lib`. Linking `jpeg-static.lib` as well would bring a second copy of `jpeg_*`. The script
checks that `jconfig.h` says `JPEG_LIB_VERSION 80`, and sets `core.autocrlf` to false so the headers are the same on
every machine. Its headers match the earlier hand build byte for byte.

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

Results, Python recipe (2026-11-13, from an empty folder): all checks passed in 369 s. The link check printed the four
expected lines, the library majors are 61 (avutil), 63 (avcodec, avformat), 10 (swscale) and 7 (swresample), the same
as the shipped DLLs, and no library references `LIBCMT`. The link check imports only system DLLs. `avcodec.lib` is
10 MB, down from 29 MB in the pilot, since `--disable-debug` leaves out the debug data.

Results, PowerShell recipe (run 1, 2026-11-08): all checks passed in 243 s. The link needs `user32.lib`
(`GetDesktopWindow`, from the DXVA2 device code) and `bcrypt.lib` (`av_get_random_seed`). FFmpeg's `.pc` files list
neither, because their `Libs.private` is empty, so the recipe carries the list itself. The H.264 decoder and the
`d3d11va` and `dxva2` device types are found. AV1 is not enabled, as intended.

## Qt

`python tools/build_static_qt.py --jpeg <jpeg>\prefix --ffmpeg <ffmpeg>\prefix [--work <dir>] [--configure-only]`.
The defaults are `PG_HOME\static\qt` for the work folder and `PG_HOME\static\jpeg-turbo\prefix` and
`PG_HOME\static\ffmpeg\prefix` for the inputs. The recipe:

1. downloads the pinned source from download.qt.io and checks its size and SHA-256 (a kept download is checked again);
2. checks the two prefixes (the JPEG 8.0 headers and `turbojpeg-static.lib`; the FFmpeg headers and `avcodec.lib`);
3. configures out of tree, builds, and installs to `<work>\prefix` (`--configure-only` stops after configure);
4. checks the install: no DLL, `bin\qmake.exe -v` reports 6.12.0, and each plugin folder holds a static library;
5. writes `manifest.json` with the source hash, the tools, and the SHA-256 of every library.

The work folder and the inputs must not have a space in their path: the configure's arguments are not quoted.

The pilot's own scripts (`scripts\qt-configure.ps1` and `scripts\qt-build.ps1`) were in a work folder that is not on
this machine, and they are not in git history. The recipe replaces them. It was written from the pilot's recorded
configure line, and it was checked by running it, as described below.

The configure, as the recipe runs it (`<work>\logs\configure.log`, 2026-11-13):

    configure.bat -prefix <work>\prefix -static -release -opensource -confirm-license -nomake tests -nomake examples
      -no-openssl -schannel -ffmpeg-dir <ffmpeg>\prefix -system-libjpeg -feature-system-jpeg
      -submodules qtbase,qtshadertools,qtdeclarative,qtmultimedia,qtimageformats,qtsvg
      -- -DJPEG_INCLUDE_DIR=<jpeg>/include -DJPEG_LIBRARY_RELEASE=<jpeg>/lib/turbojpeg-static.lib

- `-no-openssl -schannel`: TLS is Schannel, as the deployed app's `tls\qschannelbackend.dll` is. The kit's OpenSSL is
  not used.
- `-ffmpeg-dir` is the static FFmpeg prefix. The pilot used an earlier run's prefix.
- The arguments after `--` go to CMake. They set the JPEG paths for `-system-libjpeg`. Their forward slashes matter: a
  backslash in `JPEG_LIBRARY_RELEASE` reaches the generated CMake as a `set()` with an escape sequence, and fails.
- `-feature-system-jpeg` is needed. Qt's `system-jpeg` condition tests `JPEG_FOUND`, which Qt sets in its own scope, so
  the condition is false even though the library is found (version 80). `-feature-` forces the feature on.
- Qt's summary warns that static linking disables dynamically loaded plugins. Every plugin the app uses must be
  imported by name.
- The summary (`build\config.summary`) reports Schannel, JPEG with the system libjpeg, WebP and the FFmpeg multimedia
  backend, all `yes`.
- Qt's configure needs Perl. The recipe puts MSYS2's `usr\bin` on `PATH` after MSVC's folder, as the FFmpeg recipe does,
  so that MSYS2's `link` does not shadow MSVC's.

Build: `cmake --build . --parallel <jobs>`, then `cmake --install .` to `<work>\prefix`. The pilot took about 50 minutes
from configure to install (8,452 steps, none failed). `prefix\bin` has no DLLs, and `qmake -v` reports 6.12.0.

**Not run by the recipe yet.** The configure stage ran on 2026-11-13 and passed. The build, install and install checks
(steps 3 and 4 above) are written but have not been run: a full build takes close to an hour, and the pilot already
built and checked this Qt. Running them is the first step when the recipe is next needed.

Plugins are static libraries: `platforms` (`qwindows`, `qoffscreen`), `imageformats` (`qwebp` among them), `multimedia`
(`ffmpegmediaplugin`, `windowsmediaplugin`), `tls` (`qschannelbackend`), `networkinformation`, `iconengines` (`qsvgicon`).

### Link check and playback (pilot)

The pilot's link check is a small Qt program (`linkcheck\`, not in the repo) that links against `qt\prefix` and imports
the app's plugin set by name: `qwindows`, `qoffscreen`, the image formats, `qschannelbackend`, the FFmpeg and Windows
media backends, `qsvgicon`.

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
- `tools/static-qt/pg-video.patch`: `crates/video/build.rs` and `crates/video/src/ffi/load.rs`. With `PG_FFMPEG_STATIC`
  set, the build links the FFmpeg archives and skips the DLL copy, and the loader's function table points at the linked
  functions.

To apply them, from the repository root:

    git apply tools/static-qt/app-build.patch
    git apply tools/static-qt/pg-video.patch

To undo them, apply each with `-R` in reverse order, or run `git checkout -- crates/desktop crates/video` when nothing
else in those folders is changed.

The patches were rebased onto `main` on 2026-11-13 (commit `0364a4d`, formatting in `4bf619a`). Their old base had moved
in three places: `ffi.rs` split into `ffi/load.rs` (the loader), `gui_app::new` became `pub(crate)`, and pg-video's
`build.rs` gained per-OS FFmpeg headers. The static branches were rewritten against those, and the loader's dynamic
half is compiled out under `pg_ffmpeg_static`.

Checks on 2026-11-13, in a clean checkout of the commit the patches were rebased onto (`43ee58a`), with `QT_DIR` set to
the shared Qt 6.12.0 kit:

- `cargo check -p pg-video`: passes, shared and with `PG_FFMPEG_STATIC` set.
- `cargo check -p pg-desktop`: passes, shared.
- `cargo rustc -p pg-desktop -- --cfg pg_static_qt`: passes. This typechecks the static-Qt Rust code. It does not run
  `build.rs`'s static branch, which needs the static Qt.

Build, with a separate target folder:

    cargo build -p pg-gallery --bin gallery      # debug, as the pilot verified it

with these environment variables:

- `PG_QT_STATIC=1`: only this build sets it. The shared build's `build.rs` path is unchanged.
- `QT_DIR`: the static Qt prefix.
- `PG_FFMPEG_STATIC`: the static FFmpeg's `lib` folder.
- `FFMPEG_DIR`: the static FFmpeg prefix, for its headers.
- `TURBOJPEG_SOURCE=explicit`, `TURBOJPEG_STATIC=1`, `TURBOJPEG_LIB_DIR=<jpeg work>\link`,
  `TURBOJPEG_INCLUDE_DIR=<jpeg work>\prefix\include`.
- `CARGO_TARGET_DIR`: a separate folder.

Problems the pilot found by running the binary offscreen, in the order they were hit, with the fixes in the patch:

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

Verified in the pilot: `gallery.exe` (debug) starts offscreen, the QML loads, and it runs for 90 s with no JPEG error.
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
  `docs/webclient/WEB_CLIENT_DESIGN.md`.

## Pitfalls

Each of these was hit while building, and the recipes now handle it.

- **MSYS2 rewrites flags.** `/MD` and `/Brepro` become paths. Use `-MD` and `-Brepro`.
- **MSYS2's `link` shadows MSVC's.** MSYS2's coreutils `link` is found first unless MSVC's `bin` comes before `usr\bin` on
  `PATH`. The recipes put MSVC first.
- **No `cmp` in MSYS2's base.** Configure prints `cmp: command not found` on every comparison and still exits 0. Install
  `diffutils`, and read the configure log: a zero exit does not prove the configure ran.
- **Generated `.sh` files need LF line endings.** A CRLF script fails in bash. The recipe writes LF.
- **PowerShell reads `"$Toolset:"` as a drive-qualified name.** Write `${Toolset}:` (PowerShell recipe only).
- **Git converts line endings.** Git for Windows defaults to `core.autocrlf=true`, which put CRLF into the JPEG headers.
  The recipe sets it to false.
- **Qt's JPEG path needs forward slashes**, as above.
- **Spaces in paths.** The Qt configure's arguments are not quoted, so the Qt recipe refuses a work folder or input with a
  space.
- **`vswhere.exe is not recognized`** is printed by `vcvars64.bat` itself. It is harmless.

## Open items

1. **The static link is not run.** It needs a static Qt, which is the pilot's build (not yet reproduced by the recipe).
   With that Qt, build `gallery.exe` with the environment above and run it offscreen: the plugins start, the QML loads,
   and an image opens with no JPEG error.
2. **The Qt recipe's build, install and install checks are not run.** See the Qt section. The first run of them will show
   whether the plugin-folder and qmake checks match the pilot's install.
3. **The PowerShell originals** (`tools/build-static-ffmpeg.ps1`, `tools/build-jpeg-turbo.ps1`) are still in `tools/`.
   `tools/README.md` keeps them until a Windows run confirms the Python ports. Both ports have now run on 2026-11-13, so
   the originals can go once that is agreed.
4. **Codec lists are unverified.** The demuxer, decoder and parser lists are the pilot's guess. Check them against what
   pg-video needs.
5. **Not checked:** the `networkinformation` plugin (`qnetworklistmanager`) is in the app but not in the link check's
   list. Add it if the app uses `QNetworkInformation`.
6. **Not recorded:** a release build and a `windows` platform run of `gallery.exe`.
7. **Reproducibility across folders** is not met, and the same-folder standard is not met for the timestamp.
   Qt is unchecked.
