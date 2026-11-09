#!/bin/bash
# The system setup for CI's Linux job (.github/workflows/ci.yml): the apt packages (docker/apt-packages.txt, the list the
# container installs too), mold, the Rust components. FFmpeg's headers come from its development packages, which pkg-config finds. Idempotent.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
SUDO=$([ "$(id -u)" = 0 ] || echo sudo)
export DEBIAN_FRONTEND=noninteractive

# Nothing here is silenced: each stage prints its own output, so a slow one shows where the time goes.
echo "== apt: package index"
# A distro with foreign architectures enabled gets 404s for their indexes; amd64's still update.
$SUDO apt-get update || echo "apt-get update: some indexes failed (ignored)"
# libssl-dev: ureq's native-tls is OpenSSL on Linux. libclang-dev: bindgen (pg-video's build script). ffmpeg: the command
# the video tests run (pg_library's video tests look for it on PATH).
read_list() { grep -v '^#' "$1" | grep -v '^$' || true; }
mapfile -t packages < <(read_list "$REPO/docker/apt-packages.txt")
# qt-build-utils links with lld, gold or mold when one is on PATH (in that order). Without one it warns on every build
# script run that GNU ld.bfd will likely fail (it does not fail). mold needs GCC 12 or newer, and the runner has GCC 15.
packages+=(mold)
echo "== apt: ${#packages[@]} packages"
$SUDO apt-get install -y "${packages[@]}"

echo "== rustup"
[ -x "$HOME/.cargo/bin/rustup" ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
rustup component add clippy rustfmt  # the minimal profile has neither

echo "$(rustc --version | cut -d' ' -f1-2), FFmpeg libavcodec $(pkg-config --modversion libavcodec)"
