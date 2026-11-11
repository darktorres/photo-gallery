#!/usr/bin/env python3
"""The CPU side of docs/JPEG_DECODERS.md, on Linux (it was wsl-build.sh): the decoders and encoders under JPEG_EVAL
(default ~/jpeg-eval), and the harness (harness/) with libjpeg-turbo, parajpeg, jpegli and Wuffs. Usage, from the
repository: `just linux-eval eval/jpeg-decoders/linux_build.py [STEP...]` (the container, whose pg-eval volume keeps the
builds), or natively:
    python3 eval/jpeg-decoders/linux_build.py [STEP...]
Steps (default: all): turbo mozjpeg jpegli guetzli wuffs parajpeg harness. Sources are cloned once at the pinned commits
below; the command-line tools land in JPEG_EVAL/bin. Natively it needs the apt packages of docker/apt-packages.txt and
docker/apt-packages-eval.txt, which the container's eval image has.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

from linux import EVAL as E

HERE = Path(__file__).resolve().parent
JOBS = str(os.cpu_count() or 1)
ENV = {**os.environ, "CC": "clang", "CXX": "clang++"}


class BuildError(Exception):
    """A build step that failed; its message is shown, and the run stops."""


def run_logged(command: list[str], log: Path, cwd: Path | None = None) -> None:
    """Runs a build command with its output in `log`. On failure, raises with the log's tail (the errors, for a build)."""
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w", encoding="utf-8") as file:
        result = subprocess.run(command, stdout=file, stderr=subprocess.STDOUT, cwd=cwd, env=ENV, check=False)
    if result.returncode != 0:
        lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
        raise BuildError("\n".join(lines[-30:]))


def ensure_ninja() -> None:
    if shutil.which("ninja") is None:
        sys.exit("ninja is missing: install ninja-build (docker/apt-packages-eval.txt), or run in the container")


def clone(name: str, url: str, commit: str, submodules: bool = False) -> None:
    """Fetches `commit` of `url` into the sources folder, once. Applies the folder's patch (patches/<name>.patch) if it has
    one."""
    folder = E / "src" / name
    if not folder.is_dir():
        folder.mkdir(parents=True)
        subprocess.run(["git", "init", "-q", str(folder)], check=True)
        subprocess.run(["git", "-C", str(folder), "fetch", "-q", "--depth", "1", url, commit], check=True)
        subprocess.run(["git", "-C", str(folder), "checkout", "-q", "FETCH_HEAD"], check=True)
        if submodules:
            subprocess.run(
                ["git", "-C", str(folder), "submodule", "update", "-q", "--init", "--recursive", "--depth", "1"], check=True
            )
        patch = HERE / "patches" / f"{name}.patch"
        if patch.is_file():
            subprocess.run(["git", "-C", str(folder), "apply", str(patch)], check=True)
    head = subprocess.run(
        ["git", "-C", str(folder), "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=True
    ).stdout.strip()
    date = subprocess.run(
        ["git", "-C", str(folder), "log", "-1", "--format=%cs"], capture_output=True, text=True, check=True
    ).stdout.strip()
    print(f"{name:<14} {head} {date}")


def configure(name: str, source: Path, *args: str) -> None:
    """Release, Ninja, out of tree."""
    run_logged(
        ["cmake", "-G", "Ninja", "-S", str(source), "-B", str(E / "build" / name), "-DCMAKE_BUILD_TYPE=Release", *args],
        E / "build" / f"{name}.configure.log",
    )


def build(name: str, *targets: str) -> None:
    """Builds `targets`, showing the errors on failure."""
    log = E / "build" / f"{name}.build.log"
    try:
        run_logged(["cmake", "--build", str(E / "build" / name), "-j", JOBS, "--target", *targets], log)
    except BuildError:
        errors = [
            line for line in log.read_text(encoding="utf-8", errors="replace").splitlines() if "error" in line or "FAILED" in line
        ]
        raise BuildError("\n".join(errors[:20])) from None


def install(source: Path, destination: Path) -> None:
    shutil.copy2(source, destination)


def build_turbo() -> None:
    """The app's libjpeg-turbo 3.1.0, in the turbojpeg-sys fork it pins: cjpeg, djpeg, jpegtran, tjbench, libturbojpeg.a."""
    clone("rust-turbojpeg", "https://github.com/darktorres/rust-turbojpeg", "945d853b47a76f98fd97ac5ff43830997712caa2")
    configure(
        "libjpeg-turbo",
        E / "src" / "rust-turbojpeg" / "turbojpeg-sys" / "libjpeg-turbo",
        "-DENABLE_SHARED=OFF",
        "-DWITH_TURBOJPEG=ON",
    )
    build("libjpeg-turbo", "cjpeg-static", "djpeg-static", "jpegtran-static", "tjbench-static", "turbojpeg-static")
    for tool in ["cjpeg", "djpeg", "jpegtran", "tjbench"]:
        install(E / "build" / "libjpeg-turbo" / f"{tool}-static", E / "bin" / f"{tool}-turbo")


def build_mozjpeg() -> None:
    """cjpeg with trellis quantization and the optimized scan scripts."""
    clone("mozjpeg", "https://github.com/mozilla/mozjpeg", "08265790774cd0714832c9e675522acbe5581437")
    configure("mozjpeg", E / "src" / "mozjpeg", "-DBUILD_SHARED_LIBS=OFF", "-DPNG_SUPPORTED=ON")  # master renamed ENABLE_SHARED
    build("mozjpeg", "cjpeg-static", "djpeg-static", "jpegtran-static")
    for tool in ["cjpeg", "djpeg", "jpegtran"]:
        install(E / "build" / "mozjpeg" / f"{tool}-static", E / "bin" / f"{tool}-mozjpeg")


def build_jpegli() -> None:
    """google/jpegli: cjpegli, djpegli, ssimulacra2 (the encoder metric), libjpegli-static.a."""
    clone("jpegli", "https://github.com/google/jpegli", "031a0077f5799a6041004267fc12b956c1f52a20", submodules=True)
    configure("jpegli", E / "src" / "jpegli", "-DBUILD_TESTING=OFF", "-DJPEGLI_ENABLE_DEVTOOLS=ON")
    build("jpegli", "cjpegli", "djpegli", "ssimulacra2", "jpegli-static")
    for tool in ["cjpegli", "djpegli", "ssimulacra2"]:
        found = next((p for p in (E / "build" / "jpegli").rglob(tool) if p.is_file() and os.access(p, os.X_OK)), None)
        if found is None:
            raise BuildError(f"the build made no {tool}")
        install(found, E / "bin" / tool)


def build_guetzli() -> None:
    clone("guetzli", "https://github.com/google/guetzli", "214f2bb42abf5a577c079d00add5d6cc470620d3")
    try:
        run_logged(
            ["make", "-C", str(E / "src" / "guetzli"), "-j", JOBS, "config=release", "guetzli"], E / "build" / "guetzli.build.log"
        )
    except BuildError:
        log = E / "build" / "guetzli.build.log"
        errors = [line for line in log.read_text(encoding="utf-8", errors="replace").splitlines() if "error" in line]
        raise BuildError("\n".join(errors[:20])) from None
    install(E / "src" / "guetzli" / "bin" / "Release" / "guetzli", E / "bin" / "guetzli")


def build_wuffs() -> None:
    """The harness compiles Wuffs' single-file release itself; convert-to-nia is its command-line decoder."""
    clone("wuffs", "https://github.com/google/wuffs", "d28ecd494491baa9b11d2557d10819111050fd24")
    subprocess.run(
        [
            ENV["CC"],
            "-O3",
            "-o",
            str(E / "bin" / "wuffs-convert-to-nia"),
            str(E / "src" / "wuffs" / "example" / "convert-to-nia" / "convert-to-nia.c"),
        ],
        check=True,
    )


def build_parajpeg() -> None:
    """The CPU backend (the CUDA build is win_build.py's): libparajpeg.a and pjdec."""
    clone("parajpeg", "https://github.com/anniigr/parajpeg", "f45081d7f75fdab809e7170c15fd806f42deb375")
    configure(
        "parajpeg",
        E / "src" / "parajpeg",
        "-DPARAJPEG_WITH_CUDA=OFF",
        "-DPARAJPEG_BUILD_TESTS=OFF",
        "-DPARAJPEG_BUILD_BENCH=OFF",
        "-DPARAJPEG_BUILD_TOOLS=ON",
    )
    build("parajpeg", "parajpeg", "pjdec")
    install(E / "build" / "parajpeg" / "pjdec", E / "bin" / "pjdec")


def build_harness() -> None:
    turbo = E / "src" / "rust-turbojpeg" / "turbojpeg-sys" / "libjpeg-turbo"
    configure(
        "harness",
        HERE / "harness",
        f"-DTURBO_INCLUDE={turbo / 'src'};{E / 'build' / 'libjpeg-turbo'}",
        f"-DTURBO_LIB={E / 'build' / 'libjpeg-turbo' / 'libturbojpeg.a'}",
        f"-DPARAJPEG_SRC={E / 'src' / 'parajpeg'}",
        f"-DPARAJPEG_LIB={E / 'build' / 'parajpeg' / 'libparajpeg.a'}",
        f"-DJPEGLI_SRC={E / 'src' / 'jpegli'}",
        f"-DJPEGLI_BUILD={E / 'build' / 'jpegli'}",
        f"-DWUFFS_SRC={E / 'src' / 'wuffs'}",
    )
    build("harness", "harness")
    install(E / "build" / "harness" / "harness", E / "bin" / "harness")


BUILDERS = {
    "turbo": build_turbo,
    "mozjpeg": build_mozjpeg,
    "jpegli": build_jpegli,
    "guetzli": build_guetzli,
    "wuffs": build_wuffs,
    "parajpeg": build_parajpeg,
    "harness": build_harness,
}


def main() -> None:
    steps = sys.argv[1:] or list(BUILDERS)
    for step in steps:
        if step not in BUILDERS:
            sys.exit(f"unknown step {step}: one of {' '.join(BUILDERS)}")
    (E / "src").mkdir(parents=True, exist_ok=True)
    (E / "build").mkdir(parents=True, exist_ok=True)
    (E / "bin").mkdir(parents=True, exist_ok=True)
    ensure_ninja()
    for step in steps:
        try:
            BUILDERS[step]()
        except BuildError as error:
            sys.exit(f"{step}: the build failed:\n{error}")
    for path in sorted((E / "bin").iterdir()):
        print(f"{path.stat().st_size:>12} {path}")


if __name__ == "__main__":
    main()
