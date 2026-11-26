#!/usr/bin/env python3
"""The CPU side of the JPEG decoder comparison, on Linux: the decoders and encoders under PG_JPEG_EVAL
(default ~/jpeg-eval), and the harness (harness/) with libjpeg-turbo, parajpeg, jpegli and Wuffs. Usage, from the
repository: `just linux-eval eval/jpeg-decoders/linux_build.py [STEP...]` (the container, whose pg-eval volume keeps the
builds), or natively:
    python3 eval/jpeg-decoders/linux_build.py [STEP...]
Steps (default: all): turbo mozjpeg jpegli guetzli wuffs parajpeg harness. Sources are cloned once at the pinned commits
below; the command-line tools land in PG_JPEG_EVAL/bin. Natively it needs the apt packages of docker/apt-packages.txt and
docker/apt-packages-eval.txt, which the container's eval image has.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

import sources

from linux import EVAL as E

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "tools"))
from build_jpeg_turbo import FORK_COMMIT, FORK_URL  # noqa: E402 (after the sys.path insert above): the app's libjpeg-turbo pin

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


def error_lines(log: Path) -> str:
    """The error lines of a failed build's log (the compiler's, CMake's and make's), 20 at most."""
    lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join([line for line in lines if "error" in line or "FAILED" in line][:20])


def ensure_ninja() -> None:
    if shutil.which("ninja") is None:
        sys.exit("ninja is missing: install ninja-build (docker/apt-packages-eval.txt), or run in the container")


def clone(name: str, url: str, commit: str, submodules: bool = False) -> None:
    """A source checkout under the sources folder (sources.clone, shared with win_build.py)."""
    sources.clone(E / "src" / name, url, commit, submodules)


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
        raise BuildError(error_lines(log)) from None


def install(source: Path, destination: Path) -> None:
    shutil.copy2(source, destination)


def build_turbo() -> None:
    """The app's libjpeg-turbo 3.1.0, in the turbojpeg-sys fork it pins: cjpeg, djpeg, jpegtran, tjbench, libturbojpeg.a."""
    clone("rust-turbojpeg", FORK_URL, FORK_COMMIT)
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
    clone("mozjpeg", *sources.PINS["mozjpeg"])
    configure("mozjpeg", E / "src" / "mozjpeg", "-DBUILD_SHARED_LIBS=OFF", "-DPNG_SUPPORTED=ON")  # master renamed ENABLE_SHARED
    build("mozjpeg", "cjpeg-static", "djpeg-static", "jpegtran-static")
    for tool in ["cjpeg", "djpeg", "jpegtran"]:
        install(E / "build" / "mozjpeg" / f"{tool}-static", E / "bin" / f"{tool}-mozjpeg")


def build_jpegli() -> None:
    """google/jpegli: cjpegli, djpegli, ssimulacra2 (the encoder metric), libjpegli-static.a."""
    clone("jpegli", *sources.PINS["jpegli"], submodules=True)
    configure("jpegli", E / "src" / "jpegli", "-DBUILD_TESTING=OFF", "-DJPEGLI_ENABLE_DEVTOOLS=ON")
    build("jpegli", "cjpegli", "djpegli", "ssimulacra2", "jpegli-static")
    for tool in ["cjpegli", "djpegli", "ssimulacra2"]:
        found = next((p for p in (E / "build" / "jpegli").rglob(tool) if p.is_file() and os.access(p, os.X_OK)), None)
        if found is None:
            raise BuildError(f"the build made no {tool}")
        install(found, E / "bin" / tool)


def build_guetzli() -> None:
    clone("guetzli", *sources.PINS["guetzli"])
    try:
        run_logged(
            ["make", "-C", str(E / "src" / "guetzli"), "-j", JOBS, "config=release", "guetzli"], E / "build" / "guetzli.build.log"
        )
    except BuildError:
        raise BuildError(error_lines(E / "build" / "guetzli.build.log")) from None
    install(E / "src" / "guetzli" / "bin" / "Release" / "guetzli", E / "bin" / "guetzli")


def build_wuffs() -> None:
    """The harness compiles Wuffs' single-file release itself; convert-to-nia is its command-line decoder."""
    clone("wuffs", *sources.PINS["wuffs"])
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
    clone("parajpeg", *sources.PINS["parajpeg"])
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
