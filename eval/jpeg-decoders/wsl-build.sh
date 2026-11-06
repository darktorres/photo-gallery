#!/bin/bash
# The CPU side of docs/JPEG_DECODERS.md in WSL (Ubuntu-26.04): decoders and encoders under ~/jpeg-eval, and the
# harness (harness/) with libjpeg-turbo, parajpeg, jpegli and Wuffs. From the repo, in PowerShell:
#   wsl -d Ubuntu-26.04 -u root -- bash eval/jpeg-decoders/wsl-build.sh [STEP...]
# Steps (default: all): turbo mozjpeg jpegli guetzli wuffs parajpeg harness. Sources are cloned once at the pinned
# commits below; the command-line tools land in ~/jpeg-eval/bin.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
E=$HOME/jpeg-eval
mkdir -p "$E/src" "$E/build" "$E/bin"
export CC=clang CXX=clang++
J=$(nproc)
command -v ninja >/dev/null || apt-get install -y -q ninja-build >/dev/null

clone() {  # name url commit [--recurse-submodules]
    local d=$E/src/$1
    if [ ! -d "$d" ]; then
        git init -q "$d"
        git -C "$d" fetch -q --depth 1 "$2" "$3"
        git -C "$d" checkout -q FETCH_HEAD
        [ "${4:-}" != --recurse-submodules ] || git -C "$d" submodule update -q --init --recursive --depth 1
        [ ! -f "$HERE/patches/$1.patch" ] || git -C "$d" apply "$HERE/patches/$1.patch"
    fi
    printf '%-14s %s %s\n' "$1" "$(git -C "$d" rev-parse --short HEAD)" "$(git -C "$d" log -1 --format=%cs)"
}

cm() {  # name src [cmake args]: Release, Ninja, out of tree
    cmake -G Ninja -S "$2" -B "$E/build/$1" -DCMAKE_BUILD_TYPE=Release "${@:3}" >"$E/build/$1.configure.log" 2>&1 \
        || { tail -30 "$E/build/$1.configure.log"; return 1; }
}

bld() {  # name targets...: build, showing only the errors on failure
    cmake --build "$E/build/$1" -j "$J" --target "${@:2}" >"$E/build/$1.build.log" 2>&1 \
        || { grep -E "error|FAILED" "$E/build/$1.build.log" | head -20; return 1; }
}

build_turbo() {  # the app's libjpeg-turbo 3.1.0, in the turbojpeg-sys fork it pins: cjpeg, djpeg, jpegtran, tjbench, libturbojpeg.a
    clone rust-turbojpeg https://github.com/darktorres/rust-turbojpeg 945d853b47a76f98fd97ac5ff43830997712caa2
    cm libjpeg-turbo "$E/src/rust-turbojpeg/turbojpeg-sys/libjpeg-turbo" -DENABLE_SHARED=OFF -DWITH_TURBOJPEG=ON
    bld libjpeg-turbo cjpeg-static djpeg-static jpegtran-static tjbench-static turbojpeg-static
    for t in cjpeg djpeg jpegtran tjbench; do cp "$E/build/libjpeg-turbo/$t-static" "$E/bin/$t-turbo"; done
}

build_mozjpeg() {  # cjpeg with trellis quantization and the optimized scan scripts
    clone mozjpeg https://github.com/mozilla/mozjpeg 08265790774cd0714832c9e675522acbe5581437
    cm mozjpeg "$E/src/mozjpeg" -DBUILD_SHARED_LIBS=OFF -DPNG_SUPPORTED=ON  # master renamed ENABLE_SHARED
    bld mozjpeg cjpeg-static djpeg-static jpegtran-static
    for t in cjpeg djpeg jpegtran; do cp "$E/build/mozjpeg/$t-static" "$E/bin/$t-mozjpeg"; done
}

build_jpegli() {  # google/jpegli: cjpegli, djpegli, ssimulacra2 (the encoder metric), libjpegli-static.a
    clone jpegli https://github.com/google/jpegli 031a0077f5799a6041004267fc12b956c1f52a20 --recurse-submodules
    cm jpegli "$E/src/jpegli" -DBUILD_TESTING=OFF -DJPEGLI_ENABLE_DEVTOOLS=ON
    bld jpegli cjpegli djpegli ssimulacra2 jpegli-static
    for t in cjpegli djpegli ssimulacra2; do
        cp "$(find "$E/build/jpegli" -name $t -type f -perm -u+x | head -1)" "$E/bin/"
    done
}

build_guetzli() {
    clone guetzli https://github.com/google/guetzli 214f2bb42abf5a577c079d00add5d6cc470620d3
    make -C "$E/src/guetzli" -j "$J" config=release guetzli >"$E/build/guetzli.build.log" 2>&1 \
        || { grep -E "error" "$E/build/guetzli.build.log" | head -20; return 1; }
    cp "$E/src/guetzli/bin/Release/guetzli" "$E/bin/"
}

build_wuffs() {  # the harness compiles Wuffs' single-file release itself; convert-to-nia is its command-line decoder
    clone wuffs https://github.com/google/wuffs d28ecd494491baa9b11d2557d10819111050fd24
    $CC -O3 -o "$E/bin/wuffs-convert-to-nia" "$E/src/wuffs/example/convert-to-nia/convert-to-nia.c"
}

build_parajpeg() {  # CPU backend (the CUDA build is win-build.cmd's); libparajpeg.a and pjdec
    clone parajpeg https://github.com/anniigr/parajpeg f45081d7f75fdab809e7170c15fd806f42deb375
    cm parajpeg "$E/src/parajpeg" -DPARAJPEG_WITH_CUDA=OFF -DPARAJPEG_BUILD_TESTS=OFF -DPARAJPEG_BUILD_BENCH=OFF \
        -DPARAJPEG_BUILD_TOOLS=ON
    bld parajpeg parajpeg pjdec
    cp "$E/build/parajpeg/pjdec" "$E/bin/"
}

build_harness() {
    cm harness "$HERE/harness" -DTURBO_INCLUDE="$E/src/rust-turbojpeg/turbojpeg-sys/libjpeg-turbo/src;$E/build/libjpeg-turbo" \
        -DTURBO_LIB="$E/build/libjpeg-turbo/libturbojpeg.a" \
        -DPARAJPEG_SRC="$E/src/parajpeg" -DPARAJPEG_LIB="$E/build/parajpeg/libparajpeg.a" \
        -DJPEGLI_SRC="$E/src/jpegli" -DJPEGLI_BUILD="$E/build/jpegli" -DWUFFS_SRC="$E/src/wuffs"
    bld harness harness
    cp "$E/build/harness/harness" "$E/bin/"
}

steps=("$@")
[ ${#steps[@]} -gt 0 ] || steps=(turbo mozjpeg jpegli guetzli wuffs parajpeg harness)
for s in "${steps[@]}"; do "build_$s"; done
ls -la "$E/bin"
