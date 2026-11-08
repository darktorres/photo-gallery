#!/bin/bash
# Sets what CI's Linux job sets for each step (.github/workflows/ci.yml), then runs the command it was given. The
# image's own environment (Qt, FFmpeg, the toolchain paths) is in docker/Dockerfile.
set -euo pipefail
# Qt needs a UTF-8 locale; the image has C.UTF-8 and no other.
export LANG=C.UTF-8 LC_ALL=C.UTF-8
# The video test looks for the pinned Windows build in .deps first (the repository's own .deps, from the Windows
# checkout, is mounted here), so the ffmpeg of this image is named outright.
export PG_FFMPEG=/usr/bin/ffmpeg
# libclang for bindgen (ffmpeg-sys-next): the newest LLVM the image has, as CI finds it.
LIBCLANG_PATH=$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)
export LIBCLANG_PATH
exec "$@"
