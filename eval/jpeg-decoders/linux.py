#!/usr/bin/env python3
"""The JPEG decoder comparison (docs/JPEG_DECODERS.md), on Linux: in the container (`just linux-eval
eval/jpeg-decoders/linux.py STEP...`, docker/README.md) or natively (it was wsl.sh). Usage, from the repository:
    python3 eval/jpeg-decoders/linux.py STEP...
Steps: inputs (copy the 7 files to the inputs folder), check (clippy, rustfmt), compare (output vs libjpeg-turbo), time
(median wall ms), bench (callgrind through gungraun, then the table), marginal (callgrind on whole runs, all threads: libvips
and jpeg-decoder decode in their own threads, which gungraun doesn't count).
Needs the container's eval image (or, natively, tools/profiling/ci_setup.py's toolchain and docker/apt-packages-eval.txt),
and linux_build.py's decoders in JPEG_EVAL.
Environment (the container sets each to a folder of its pg-eval volume): JPEG_EVAL (default ~/jpeg-eval, linux_build.py's
sources, builds and tools), JPEG_INPUTS (default ~/jpeg-inputs), JPEG_SRC_DIR (for `inputs`: one <name>.jpg per NAMES
entry), JPEG_RESTART_OUT (default ~/jpeg-restart, restart.py's variants), PG_JPEG_ENC_DIR (default ~/jpeg-enc,
encoders.py's work folder), PG_BENCH_CLONE (default ~/photo-gallery, a clone of this repository that the benches are built
in).
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CLONE = Path(os.environ.get("PG_BENCH_CLONE", Path.home() / "photo-gallery"))
INPUTS = Path(os.environ.get("JPEG_INPUTS", Path.home() / "jpeg-inputs"))
EVAL = Path(os.environ.get("JPEG_EVAL", Path.home() / "jpeg-eval"))
RESTART = Path(os.environ.get("JPEG_RESTART_OUT", Path.home() / "jpeg-restart"))
ENCODES = Path(os.environ.get("PG_JPEG_ENC_DIR", Path.home() / "jpeg-enc"))
# Its Cargo.toml takes turbojpeg-sys from the darktorres fork by rev (vendor/README.md).
DST = CLONE / "eval" / "jpeg-decoders"
NAMES = [
    "pin-base444-1mp",
    "pin-base444-3mp",
    "pin-prog420-1mp",
    "pin-base420-1mp",
    "phone-base420-4mp",
    "sony-base422-7mp",
    "a6000-base422-24mp",
]
FILES = [INPUTS / f"{name}.jpg" for name in NAMES]


def ffmpeg_prefix() -> Path:
    """FFmpeg's prefix for this crate's build: ffmpeg-sys-next reads FFMPEG_DIR (include/ and lib/), which the crate's .cargo
    names as .deps/ffmpeg (Windows). On Linux that is a prefix of symlinks over Ubuntu's multiarch directories."""
    prefix = Path.home() / ".local" / "share" / "pg-bench" / "ffmpeg"
    arch = subprocess.run(
        ["dpkg-architecture", "-qDEB_HOST_MULTIARCH"], capture_output=True, text=True, check=True
    ).stdout.strip()
    prefix.mkdir(parents=True, exist_ok=True)
    for link, target in (("include", f"/usr/include/{arch}"), ("lib", f"/usr/lib/{arch}")):
        path = prefix / link
        if not path.is_symlink() or path.readlink() != target:
            path.unlink(missing_ok=True)
            path.symlink_to(target)
    return prefix


def libclang_path() -> str:
    """The newest LLVM's library folder: the one bindgen loads libclang from (as CI finds it)."""
    folders = sorted(
        Path("/usr/lib").glob("llvm-*/lib"),
        key=lambda p: [int(n) if n.isdigit() else 0 for n in re.findall(r"\d+", p.parent.name)],
    )
    return str(folders[-1]) if folders else ""


def environment() -> dict[str, str]:
    env = dict(os.environ)
    env["FFMPEG_DIR"] = str(ffmpeg_prefix())
    env["LIBCLANG_PATH"] = libclang_path()
    env["JPEG_INPUTS"] = str(INPUTS)
    # The crate's own target folder, which marginal, restart.py and table.py read, whatever CARGO_TARGET_DIR says (the
    # container sets it to the workspace's).
    env["CARGO_TARGET_DIR"] = str(DST / "target")
    return env


def run(
    command: list[str],
    *,
    cwd: Path = DST,
    env: dict[str, str] | None = None,
    check: bool = True,
    quiet: bool = False,
) -> subprocess.CompletedProcess[str]:
    """Runs `command`; a failure ends this program with its status, unless `check` is false. `quiet` discards its output."""
    discard = subprocess.DEVNULL if quiet else None
    return subprocess.run(command, cwd=cwd, env=env, check=check, text=True, stdout=discard, stderr=discard)


def callgrind_total(path: Path) -> int | None:
    """The instruction total of a callgrind output file (its summary or totals line), or None."""
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith(("summary:", "totals:")):
            return int(line.split()[1])
    return None


def marginal(env: dict[str, str]) -> None:
    """Callgrind on `repeat 3` minus `repeat 1`, halved: one decode, every thread."""
    binary = DST / "target" / "release" / "jpeg-decoders"
    run(["cargo", "build", "-q", "--release"], env=env)

    def ir(count: int, decoder: str, mode: str, file: Path) -> float | str:
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "cg"
            result = run(
                [
                    "valgrind",
                    "--tool=callgrind",
                    f"--callgrind-out-file={out}",
                    str(binary),
                    "repeat",
                    str(count),
                    decoder,
                    mode,
                    str(file),
                ],
                check=False,
                env=env,
                quiet=True,
            )
            return "ERR" if result.returncode != 0 else (callgrind_total(out) or 0)

    print(f"M instructions per decode, all threads; columns: {' '.join(NAMES)}")
    for decoder in ["libjpeg-turbo", "libvips", "jpeg-decoder"]:
        for mode in ["full", "analyze scale"]:
            row = f"{decoder:<20} {mode:<14}"
            for file in FILES:
                a, b = ir(1, decoder, mode, file), ir(3, decoder, mode, file)
                if a == "ERR" or b == "ERR":
                    row += f" {'—':>9}"
                else:
                    row += f" {(b - a) / 2e6:9.1f}"  # type: ignore[operator]
            print(row)


def inputs() -> None:
    """Copies the source photos in: they are not in the repository, and $JPEG_SRC_DIR holds one <name>.jpg per NAMES entry."""
    source = os.environ.get("JPEG_SRC_DIR")
    if not source:
        sys.exit("set JPEG_SRC_DIR to a directory with <name>.jpg for each entry in NAMES")
    INPUTS.mkdir(parents=True, exist_ok=True)
    for name in NAMES:
        target = INPUTS / f"{name}.jpg"
        if not target.exists():  # cp -n: an input already there is kept
            shutil.copy2(Path(source) / f"{name}.jpg", target)
    for path in sorted(INPUTS.iterdir()):
        print(f"{path.stat().st_size:>12} {path}")


def main() -> None:
    steps = sys.argv[1:]
    if not steps:
        sys.exit(__doc__ or "usage: linux.py STEP...")
    env = environment()
    if not (CLONE / ".git").exists():
        run(["git", "clone", "-q", str(REPO), str(CLONE)], cwd=Path.home(), env=env)
    # The crate is built from a copy of the parts that build it, in the clone.
    DST.mkdir(parents=True, exist_ok=True)
    for file in ["Cargo.toml", "Cargo.lock", "build.rs"]:
        shutil.copy2(REPO / "eval" / "jpeg-decoders" / file, DST / file)
    for folder in ["src", "benches"]:
        shutil.copytree(REPO / "eval" / "jpeg-decoders" / folder, DST / folder, dirs_exist_ok=True)
    for step in steps:
        if step == "inputs":
            inputs()
        elif step == "check":
            run(["cargo", "clippy", "-q", "--all-targets", "--", "-D", "warnings"], env=env)
            run(["cargo", "fmt", "--check"], env=env)
            print("clippy, rustfmt: ok")
        elif step == "compare":
            run(["cargo", "run", "-q", "--release", "--", "compare", *map(str, FILES)], env=env)
        elif step == "time":
            run(["cargo", "run", "-q", "--release", "--", "time", "--secs", "1", *map(str, FILES)], env=env)
        elif step == "bench":
            run(["cargo", "bench", "-q", "--bench", "ir"], env=env)
            run([sys.executable, str(REPO / "eval" / "jpeg-decoders" / "table.py"), str(DST / "target" / "gungraun")])
        elif step == "marginal":
            marginal(env)
        else:
            sys.exit(f"unknown step {step}")


if __name__ == "__main__":
    main()
