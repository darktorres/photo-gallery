#!/bin/bash
# The toolchain for the callgrind benches in a WSL2 Ubuntu 26.04 distro (docs/PROFILING.md). Idempotent. From the
# repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash tools/profiling/wsl-setup.sh
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
SUDO=$([ "$(id -u)" = 0 ] || echo sudo)
export DEBIAN_FRONTEND=noninteractive

# A distro with foreign architectures enabled gets 404s for their indexes; amd64's still update.
$SUDO apt-get update -qq || echo "apt-get update: some indexes failed (ignored)"
# libssl-dev: ureq's native-tls is OpenSSL on Linux. libclang-dev: bindgen, for ffmpeg-sys-next. ffmpeg: the command
# the video tests run (pg_library's video tests look for it on PATH). The list is apt-packages.txt, which CI installs
# from too (.github/workflows/ci.yml).
mapfile -t packages < <(grep -v '^#' "$REPO/tools/profiling/apt-packages.txt" | grep -v '^$')
$SUDO apt-get install -y -qq "${packages[@]}" >/dev/null

[ -x "$HOME/.cargo/bin/rustup" ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
. "$HOME/.cargo/env"
rustup component add clippy rustfmt 2>/dev/null  # the minimal profile has neither

# The runner must be the exact version of the gungraun library in Cargo.lock.
want=$(awk '$0 == "name = \"gungraun\"" { getline; gsub(/version = |"/, ""); print }' "$REPO/Cargo.lock")
have=$(gungraun-runner --version 2>/dev/null | awk '{ print $2 }' || true)
[ "$have" = "$want" ] || cargo install -q --locked gungraun-runner --version "$want"

# .cargo/config.toml sets FFMPEG_DIR for Windows, and cargo can override an [env] entry but not unset it. So
# wsl-bench.sh points it at a prefix laid out as ffmpeg-sys-next expects (include/, lib/), over Ubuntu's multiarch
# directories.
arch=$(dpkg-architecture -qDEB_HOST_MULTIARCH)
prefix=$HOME/.local/share/pg-bench/ffmpeg
mkdir -p "$prefix"
ln -sfn "/usr/include/$arch" "$prefix/include"
ln -sfn "/usr/lib/$arch" "$prefix/lib"

# The repo on /mnt/c belongs to the Windows user; git refuses to read it otherwise.
git config --global --get-all safe.directory | grep -qxF "$REPO" || git config --global --add safe.directory "$REPO"

echo "valgrind $(valgrind --version | cut -d- -f2), $(rustc --version | cut -d' ' -f1-2), gungraun-runner $want," \
    "FFmpeg libavcodec $(pkg-config --modversion libavcodec)"
