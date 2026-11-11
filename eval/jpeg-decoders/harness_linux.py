#!/usr/bin/env python3
"""The harness's CPU decoders (docs/JPEG_DECODERS.md), after linux_build.py (it was harness-wsl.sh), on Linux: in the
container (`just linux-eval eval/jpeg-decoders/harness_linux.py STEP...`) or natively. Usage:
    python3 eval/jpeg-decoders/harness_linux.py STEP...
Steps:
  compare     output against libjpeg-turbo (time mode with no budget: the times are not meant to be read)
  ir          callgrind, every thread: (repeat 3 - repeat 1) / 2 = instructions for one decode, setup excluded
  time        median wall ms and CPU ms per decode (needs an idle machine)
  throughput  12 workers, one image each at a time: images/s and CPU ms per image (needs an idle machine)
Inputs: JPEG_RESTART_OUT (restart.py; default ~/jpeg-restart): each file as it is (.orig), the progressive one converted
losslessly to baseline (.base), and each with a restart marker every MCU row (.rstrow).
"""

import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from linux import EVAL, NAMES, RESTART, callgrind_total, run

HARNESS = EVAL / "bin" / "harness"
ORIGINALS = [f"{name}.orig.jpg" for name in NAMES]
RESTARTS = [f"{name}.rstrow.jpg" for name in NAMES]
FILES = [*ORIGINALS, "pin-prog420-1mp.base.jpg", *RESTARTS]
CPU_DECODERS = "turbo,turbo-yuv,parajpeg-t1,parajpeg-t6,parajpeg-t12,jpegli,wuffs"
IR_DECODERS = ["turbo", "turbo-yuv", "parajpeg-t1", "parajpeg-t12", "jpegli", "wuffs"]


def ir_one(decoder: str, file: str) -> str:
    """M instructions for one decode, all threads: ERR when valgrind fails, else the (repeat 3 - repeat 1) / 2 figure."""
    totals = []
    with tempfile.TemporaryDirectory() as folder:
        for count in (1, 3):
            out = Path(folder) / str(count)
            result = run(
                [
                    "valgrind",
                    "--tool=callgrind",
                    f"--callgrind-out-file={out}",
                    str(HARNESS),
                    "repeat",
                    str(count),
                    decoder,
                    file,
                ],
                cwd=RESTART,
                check=False,
                quiet=True,
            )
            if result.returncode != 0:
                return "ERR"
            totals.append(callgrind_total(out) or 0)
    return f"{(totals[1] - totals[0]) / 2e6:.1f}"


def instructions() -> None:
    print(f"M instructions per decode (callgrind, all threads); columns: {' '.join(IR_DECODERS)}")
    with ThreadPoolExecutor(
        max_workers=len(IR_DECODERS)
    ) as pool:  # one job per decoder in parallel: instruction counts don't depend on load
        for file in FILES:
            values = dict(zip(IR_DECODERS, pool.map(lambda d, f=file: ir_one(d, f), IR_DECODERS), strict=True))
            print(f"{file:<32}" + "".join(f" {values[d]:>10}" for d in IR_DECODERS))


def main() -> None:
    steps = sys.argv[1:]
    if not steps:
        sys.exit(__doc__ or "usage: harness_linux.py STEP...")
    for step in steps:
        cwd = RESTART
        if step == "compare":
            # `${CPU_DECODERS/turbo-yuv,/}`: the same list without turbo-yuv.
            run([str(HARNESS), "time", "--secs", "0", "--dec", CPU_DECODERS.replace("turbo-yuv,", "", 1), *FILES], cwd=cwd)
        elif step == "ir":
            instructions()
        elif step == "time":
            run([str(HARNESS), "time", "--secs", "2", "--dec", CPU_DECODERS, *FILES], cwd=cwd)
        elif step == "throughput":
            for decoder in ["turbo", "parajpeg-t1", "jpegli", "wuffs"]:
                run([str(HARNESS), "throughput", "--secs", "3", "--workers", "12", decoder, *ORIGINALS], cwd=cwd)
        else:
            sys.exit(f"unknown step {step}")


if __name__ == "__main__":
    main()
