#!/usr/bin/env python3
"""Restart markers and progressive -> baseline, both lossless (jpegtran), on the 7 decoder inputs (docs/JPEG_DECODERS.md
"Restart markers and lossless baseline conversion"): size, pixel identity (djpeg, full and 1/2), and libjpeg-turbo's cost
(the vendored 3.1, through the eval harness): callgrind instructions per decode (3 decodes minus 1, halved) and wall ms per
decode (best of 3 runs of N decodes). It was restart.sh. Run after `linux.py inputs` and `linux.py time`
(which copy the inputs and build the decoder binary), in the container (`just linux-eval eval/jpeg-decoders/restart.py`) or
natively:
    python3 eval/jpeg-decoders/restart.py      (natively, needs libjpeg-turbo-progs, for jpegtran and djpeg)
Environment: JPEG_INPUTS (default ~/jpeg-inputs), JPEG_RESTART_OUT (default ~/jpeg-restart), PG_BENCH_CLONE.
"""

import subprocess
import tempfile
import time
from pathlib import Path

from linux import CLONE, INPUTS, NAMES, callgrind_total
from linux import RESTART as OUT  # the variants, kept for GPU tests

BIN = CLONE / "eval" / "jpeg-decoders" / "target" / "release" / "jpeg-decoders"  # linux.py builds it
VARIANTS = ["orig", "base", "rst10b", "rstrow"]


def make_variant(name: str, variant: str) -> Path:
    """The variant of the input: as it is, baseline (optimized tables, no markers), a marker every 10 MCUs (Fastvideo's 4:4:4),
    or a marker every MCU row."""
    source = INPUTS / f"{name}.jpg"
    destination = OUT / f"{name}.{variant}.jpg"
    commands = {
        "orig": None,
        "base": ["jpegtran", "-copy", "all", "-optimize"],
        "rst10b": ["jpegtran", "-copy", "all", "-optimize", "-restart", "10B"],
        "rstrow": ["jpegtran", "-copy", "all", "-optimize", "-restart", "1"],
    }
    command = commands[variant]
    if command is None:
        destination.write_bytes(source.read_bytes())
    else:
        with destination.open("wb") as file:
            subprocess.run([*command, str(source)], stdout=file, check=True)
    return destination


def instructions(mode: str, file: Path) -> str:
    """M instructions per decode, all threads; ERR when valgrind fails (as linux.py's)."""
    totals = []
    with tempfile.TemporaryDirectory() as folder:
        for count in (1, 3):
            out = Path(folder) / f"{count}.out"
            result = subprocess.run(
                [
                    "valgrind",
                    "--tool=callgrind",
                    f"--callgrind-out-file={out}",
                    str(BIN),
                    "repeat",
                    str(count),
                    "libjpeg-turbo",
                    mode,
                    str(file),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            if result.returncode != 0:
                return "ERR"
            totals.append(callgrind_total(out) or 0)
    return f"{(totals[1] - totals[0]) / 2e6:.1f}"


def wall(mode: str, file: Path, count: int) -> str:
    """Milliseconds per decode, the best of 3 runs of `count` decodes (less sensitive to outside load)."""
    best = float("inf")
    for _ in range(3):
        start = time.perf_counter()
        subprocess.run([str(BIN), "repeat", str(count), "libjpeg-turbo", mode, str(file)], stdout=subprocess.DEVNULL, check=True)
        best = min(best, (time.perf_counter() - start) * 1000 / count)
    return f"{best:.2f}"


def decoded(file: Path, *extra: str) -> bytes:
    return subprocess.run(["djpeg", "-ppm", *extra, str(file)], capture_output=True, check=True).stdout


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    print(
        f"{'input':<20} {'variant':<7} {'bytes':>9} {'size%':>8} {'px-full':<9} {'px-1/2':<9} "
        f"{'Mir-full':>8} {'Mir-scal':>8} {'ms-full':>8} {'ms-scal':>8}"
    )
    for name in NAMES:
        source = INPUTS / f"{name}.jpg"
        original_size = source.stat().st_size
        reference_full = decoded(source)
        reference_half = decoded(source, "-scale", "1/2")
        # roughly constant work per wall measurement
        reps = 60_000_000 // original_size + 3
        for variant in VARIANTS:
            file = make_variant(name, variant)
            size = file.stat().st_size
            pixels_full = "same" if decoded(file) == reference_full else "DIFF"
            pixels_half = "same" if decoded(file, "-scale", "1/2") == reference_half else "DIFF"
            print(
                f"{name:<20} {variant:<7} {size:>9d} {(size - original_size) * 100 / original_size:+7.1f}% {pixels_full:<9} "
                f"{pixels_half:<9} {instructions('full', file):>8} {instructions('analyze scale', file):>8} "
                f"{wall('full', file, reps):>8} {wall('analyze scale', file, reps):>8}"
            )


if __name__ == "__main__":
    main()
