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
