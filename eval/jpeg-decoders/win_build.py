#!/usr/bin/env python3
"""The Windows side of docs/JPEG_DECODERS.md: GPU decoders, the vendored libjpeg-turbo, and the harness. It was win-build.cmd.

    python eval/jpeg-decoders/win_build.py [turbo jpeggpu gpujpeg parajpeg debesheedas harness]   (default: all)

Sources go to %JPEG_EVAL%\\src at pinned commits (patches\\*.patch applied on a fresh clone), builds to %JPEG_EVAL%\\build. The
toolchain comes from win_env.py: VS Build Tools, the CUDA toolkit (CUDA_PATH), CMake, Ninja, and LLVM's clang-cl (LLVM_DIR,
default %ProgramFiles%\\LLVM). NASM is not set up here: it must already be on PATH. Windows only.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "tools"))
sys.path.insert(0, str(HERE))

from win_env import Toolchain, setup  # noqa: E402

from build_jpeg_turbo import FORK_COMMIT, FORK_URL  # noqa: E402
from pgtools import run  # noqa: E402

ORDER = ["turbo", "jpeggpu", "gpujpeg", "parajpeg", "debesheedas", "harness"]


def clone(env: Toolchain, name: str, url: str, commit: str) -> None:
    """A full clone with autocrlf off (LF line endings, as upstream), checked out at `commit`, then the local patch if there is
    one. A folder already there is kept as it is."""
    dest = env.jpeg_eval / "src" / name
    if dest.exists():
        return
    run(["git", "-c", "core.autocrlf=false", "clone", "-q", url, str(dest)])
    run(["git", "-C", str(dest), "-c", "core.autocrlf=false", "checkout", "-q", commit])
    patch = HERE / "patches" / f"{name}.patch"
    if patch.is_file():
        run(["git", "-C", str(dest), "apply", str(patch)])


def turbo(env: Toolchain) -> None:
    """The app's libjpeg-turbo 3.1.0, static with SIMD: the reference and CPU baseline. Its source is the one the app
    builds, the rust-turbojpeg fork's `turbojpeg-sys/libjpeg-turbo` at the commit the root Cargo.toml pins (the pin is
    tools/build_jpeg_turbo.py's, which Qt's JPEG plugin is built from too). The repository vendored it before."""
    clone(env, "rust-turbojpeg", FORK_URL, FORK_COMMIT)
    source = env.jpeg_eval / "src" / "rust-turbojpeg" / "turbojpeg-sys" / "libjpeg-turbo"
    build = env.jpeg_eval / "build" / "libjpeg-turbo"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(source), "-B", str(build),
            "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_C_COMPILER=cl", "-DENABLE_SHARED=OFF", "-DWITH_TURBOJPEG=ON",
            f"-DCMAKE_INSTALL_PREFIX={env.jpeg_eval / 'deps' / 'libjpeg-turbo'}",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build), "-j", "4"])
    run(["cmake", "--install", str(build)], quiet=True)


def jpeggpu(env: Toolchain) -> None:
    clone(env, "jpeggpu", "https://github.com/nolmoonen/jpeggpu", "3ae7ac0dbc6e1b047abaa67badb6378baa7d39cb")
    src, build = env.jpeg_eval / "src" / "jpeggpu", env.jpeg_eval / "build" / "jpeggpu"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", *env.cuda_msvc_flags,
            "-DCMAKE_CXX_FLAGS=/Zc:preprocessor /EHsc",
        ],
        quiet=True,
    )  # fmt: skip
    # The library alone, which the harness links: the project's test program does not build with this MSVC (it passes a
    # std::filesystem::path where a narrow string is wanted), and win-build.cmd let that failure through unchecked.
    run(["cmake", "--build", str(build), "--target", "jpeggpu", "-j", "4"])


def gpujpeg(env: Toolchain) -> None:
    clone(env, "GPUJPEG", "https://github.com/CESNET/GPUJPEG", "37ab4810657fafeedad8c7a3c4546ea4ef5eb363")
    src, build = env.jpeg_eval / "src" / "GPUJPEG", env.jpeg_eval / "build" / "GPUJPEG"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", *env.cuda_msvc_flags,
            "-DBUILD_SHARED_LIBS=ON", "-DCMAKE_DISABLE_FIND_PACKAGE_OpenGL=ON", "-DCMAKE_DISABLE_FIND_PACKAGE_GLEW=ON",
            "-DCMAKE_DISABLE_FIND_PACKAGE_GLUT=ON", "-DCMAKE_DISABLE_FIND_PACKAGE_GLFW=ON",
            "-DCMAKE_C_FLAGS=/Zc:preprocessor", "-DCMAKE_CXX_FLAGS=/Zc:preprocessor /EHsc",
        ],
        quiet=True,
    )  # fmt: skip
    run(["cmake", "--build", str(build), "-j", "4"])


def parajpeg(env: Toolchain) -> None:
    """clang-cl for its C++ (GCC/Clang attributes and builtins), nvcc on cl for the .cu; the CUDA build has the CPU backend
    too. __builtin_cpu_supports needs clang's builtins runtime, which lld-link doesn't add by itself."""
    clone(env, "parajpeg", "https://github.com/anniigr/parajpeg", "f45081d7f75fdab809e7170c15fd806f42deb375")
    src, build = env.jpeg_eval / "src" / "parajpeg", env.jpeg_eval / "build" / "parajpeg-cuda"
    for version in sorted((env.llvm_dir / "lib" / "clang").glob("*")):
        if version.is_dir():
            os.environ["LIB"] = f"{version / 'lib' / 'windows'};{os.environ.get('LIB', '')}"
    run(
        [
            "cmake", "-G", "Ninja", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release",
            f"-DCMAKE_C_COMPILER={env.clang_cl}", f"-DCMAKE_CXX_COMPILER={env.clang_cl}", "-DCMAKE_CXX_FLAGS=/EHsc -O2",
            "-DCMAKE_EXE_LINKER_FLAGS=clang_rt.builtins-x86_64.lib", "-DPARAJPEG_WITH_CUDA=ON", "-DCMAKE_CUDA_HOST_COMPILER=cl",
            "-DCMAKE_CUDA_FLAGS=-Xcompiler=/Zc:preprocessor", "-DPARAJPEG_BUILD_TESTS=OFF", "-DPARAJPEG_BUILD_BENCH=OFF",
            "-DPARAJPEG_BUILD_TOOLS=ON",
        ],
        quiet=True,
    )  # fmt: skip
    # The fuzz replay tool links the kernels' CPU emulator beside the kernels (duplicate symbols on Windows): skip it.
    run(["cmake", "--build", str(build), "-j", "4", "--target", "parajpeg", "pjdec"])


def debesheedas(env: Toolchain) -> None:
    """debesheedas/GPU-JPEG-Decoder (a course project): its Makefile's one nvcc line, for this GPU's compute capability."""
    clone(env, "GPU-JPEG-Decoder", "https://github.com/debesheedas/GPU-JPEG-Decoder", "61cfb3726691fe2436a84696d6a977d5bd8c6123")
    out = env.jpeg_eval / "build" / "debesheedas"
    out.mkdir(parents=True, exist_ok=True)
    run(
        [
            "nvcc",
            "-O3",
            "-std=c++17",
            f"-arch=sm_{env.cuda_arch}",
            "--relocatable-device-code",
            "true",
            "-Xcompiler",
            "/Zc:preprocessor /EHsc /O2",
            "-o",
            str(out / "decoder.exe"),
            "main.cu",
            "src/parser.cu",
            "src/huffmanTree.cpp",
            "utils/stream.cpp",
            "utils/utils.cu",
        ],
        cwd=env.jpeg_eval / "src" / "GPU-JPEG-Decoder" / "cuda-decoder",
    )


def harness(env: Toolchain) -> None:
    """The harness, with the libraries above. Its configure output is kept, and shown when it fails."""
    src, build = env.jpeg_eval / "src", env.jpeg_eval / "build"
    log = build / "harness.configure.log"
    with log.open("w", encoding="utf-8") as file:
        result = subprocess.run(
            [
                "cmake",
                "-G",
                "Ninja",
                "-S",
                str(HERE / "harness"),
                "-B",
                str(build / "harness"),
                "-DCMAKE_BUILD_TYPE=Release",
                "-DCMAKE_CXX_COMPILER=cl",
                "-DWITH_CUDA=ON",
                f"-DTURBO_INCLUDE={env.jpeg_eval / 'deps' / 'libjpeg-turbo' / 'include'}",
                f"-DTURBO_LIB={env.jpeg_eval / 'deps' / 'libjpeg-turbo' / 'lib' / 'turbojpeg-static.lib'}",
                f"-DPARAJPEG_SRC={src / 'parajpeg'}",
                f"-DPARAJPEG_LIB={build / 'parajpeg-cuda' / 'parajpeg.lib'}",
                f"-DJPEGGPU_SRC={src / 'jpeggpu'}",
                f"-DJPEGGPU_BUILD={build / 'jpeggpu'}",
                f"-DGPUJPEG_SRC={src / 'GPUJPEG'}",
                f"-DGPUJPEG_BUILD={build / 'GPUJPEG'}",
            ],
            stdout=file,
            stderr=subprocess.STDOUT,
            check=False,
        )
    if result.returncode != 0:
        print(log.read_text(encoding="utf-8"))
        sys.exit(result.returncode)
    run(["cmake", "--build", str(build / "harness"), "-j", "4"])
    # Next to its DLLs: GPUJPEG's (CUDA's nvJPEG and cudart come from the toolkit's bin on PATH).
    shutil.copy(build / "GPUJPEG" / "gpujpeg.dll", build / "harness")


STEPS = {
    "turbo": turbo,
    "jpeggpu": jpeggpu,
    "gpujpeg": gpujpeg,
    "parajpeg": parajpeg,
    "debesheedas": debesheedas,
    "harness": harness,
}


def main() -> None:
    steps = sys.argv[1:] or ORDER
    unknown = [step for step in steps if step not in STEPS]
    if unknown:
        sys.exit(f"win_build.py: unknown step(s) {' '.join(unknown)}; the steps are {' '.join(ORDER)}")
    env = setup()
    for folder in [env.jpeg_eval / "src", env.jpeg_eval / "build"]:
        folder.mkdir(parents=True, exist_ok=True)
    for step in steps:
        STEPS[step](env)


if __name__ == "__main__":
    main()
