#!/usr/bin/env python3
"""The build environment of the GPU decoder builds on Windows (docs/JPEG_DECODERS.md "To rerun"): MSVC from the Visual Studio
Build Tools (found by vswhere), the CUDA toolkit, and the paths win_build.py reads. It was win-env.cmd, which win-build.cmd
called; win_build.py calls setup() here. The repo's .env supplies the keys the environment does not set (PG_ROOT, LLVM_DIR,
CUDA_ARCH, JPEG_EVAL, and CUDA_PATH when NVIDIA's installer has not set it in this shell). Windows only.
"""

import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tools"))

from pgtools import import_msvc_environment, load_env_file  # noqa: E402


@dataclass(frozen=True)
class Toolchain:
    jpeg_eval: Path  # the folder for sources (src), builds (build) and the libjpeg-turbo install (deps)
    cuda_arch: str  # the compute capability without the dot (75 for 7.5)
    cuda_msvc_flags: list[str]  # CMake arguments: CUDA 13's CCCL headers need MSVC's conforming preprocessor
    llvm_dir: Path
    clang_cl: Path  # for code that needs GCC/Clang extensions (parajpeg): clang-cl for C++, nvcc still on cl


def nvidia_compute_capability() -> str | None:
    """The first GPU's compute capability without the dot (7.5 -> 75), or None when nvidia-smi reports none."""
    try:
        result = subprocess.run(
            ["nvidia-smi", "--query-gpu=compute_cap", "--format=csv,noheader"],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if result.returncode != 0 or not result.stdout.strip():
        return None
    major, _, minor = result.stdout.split()[0].partition(".")
    return f"{major}{minor}" if minor else None


def setup() -> Toolchain:
    """Sets this process's environment for the builds (MSVC, CUDA on PATH) and returns the settings they use. A missing
    requirement ends this program with the message that says how to supply it."""
    if os.name != "nt":
        sys.exit("win_env: Windows only (MSVC and the CUDA toolkit)")
    load_env_file()
    installer = Path(os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio" / "Installer"
    vswhere = installer / "vswhere.exe"
    if not vswhere.is_file():
        sys.exit("win_env: vswhere.exe not found in the Visual Studio Installer folder")
    found = subprocess.run(
        [str(vswhere), "-products", "*", "-latest", "-property", "installationPath"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split("\n")
    installation = [line.strip() for line in found if line.strip()]
    if not installation:
        sys.exit("win_env: vswhere found no Visual Studio installation with the C++ tools")
    # vcvars64.bat runs vswhere.exe by its bare name, which resolves only with the Installer folder on PATH.
    os.environ["PATH"] = os.pathsep.join([str(installer), os.environ.get("PATH", "")])
    import_msvc_environment(Path(installation[-1]))

    cuda = os.environ.get("CUDA_PATH")
    if not cuda:
        sys.exit("win_env: CUDA_PATH is not set; NVIDIA's CUDA installer sets it, or set it in .env")
    os.environ["PATH"] = os.pathsep.join([f"{cuda}\\bin", f"{cuda}\\bin\\x64", os.environ.get("PATH", "")])

    # CUDA_ARCH is the compute capability without the dot. Unset, it comes from the first GPU.
    cuda_arch = os.environ.get("CUDA_ARCH") or nvidia_compute_capability()
    if not cuda_arch:
        sys.exit("win_env: CUDA_ARCH is not set and nvidia-smi reported no compute capability")

    llvm_dir = Path(os.environ.setdefault("LLVM_DIR", str(Path(os.environ.get("PROGRAMFILES", r"C:\Program Files")) / "LLVM")))
    jpeg_eval = os.environ.get("JPEG_EVAL")
    if not jpeg_eval and os.environ.get("PG_ROOT"):
        jpeg_eval = str(Path(os.environ["PG_ROOT"]) / "tools" / "jpeg-eval")
    if not jpeg_eval:
        sys.exit("win_env: JPEG_EVAL or PG_ROOT must be set, for example in the repo's .env")

    return Toolchain(
        jpeg_eval=Path(jpeg_eval),
        cuda_arch=cuda_arch,
        cuda_msvc_flags=[
            "-DCMAKE_C_COMPILER=cl",
            "-DCMAKE_CXX_COMPILER=cl",
            f"-DCMAKE_CUDA_ARCHITECTURES={cuda_arch}",
            "-DCMAKE_CUDA_FLAGS=-Xcompiler=/Zc:preprocessor",
        ],
        llvm_dir=llvm_dir,
        clang_cl=llvm_dir / "bin" / "clang-cl.exe",
    )
