"""Encode time for docs/reference/JPEG_ENCODERS.md "ImageReduce's switch": ImageReduce's libvips jpegsave (Q75, 4:4:4,
optimize_coding) against cjpegli baseline 4:4:4 (Q80), on the same pixels, as imagereduce_switch.py's run CSVs select
them. Encode only: the pixels are in memory (vips) or a PPM on disk (cjpegli) before the timer starts. cjpegli runs as
a process per file, so its wall time includes process start; the 8x8 baseline is subtracted for the "net" figure.

    python eval/jpeg-decoders/imagereduce_timing.py WORK --n 500

WORK holds library.csv and camera.csv from imagereduce_switch.py's run; timing.csv is written there. Settings as in
imagereduce_switch.py (IMAGEREDUCE_VIPS_BIN, JPEGLI_BUILD). VIPS_CONCURRENCY is 1; cjpegli's threads can't be set from
its command line, so its wall time may use more cores.
"""

import argparse
import csv
import os
import random
import statistics
import subprocess
import time
from pathlib import Path

os.environ["VIPS_CONCURRENCY"] = "1"  # set before pyvips loads
import imagereduce_switch as s  # sets PATH for libvips and jpegli; the harness folder is on sys.path

REPS = 3
JP_Q = 80
FIELDS = ["set", "w", "h", "mpx", "vips_ms", "jp_ms", "jp_ms_net"]


def load_px(path: Path):
    """The pixels ImageReduce would encode, materialized so the timer sees only the encode."""
    data = path.read_bytes()
    img = s.pyvips.Image.new_from_buffer(data, "", access="sequential")
    base, _ = s.thumbnail_or_full(data, img.width, img.height)
    px = base.colourspace("srgb") if base.interpretation != "srgb" else base
    if px.format != "uchar":
        px = px.cast("uchar")
    return px.copy_memory()


def median_time(fn, reps: int = REPS) -> float:
    times = []
    for _ in range(reps):
        t0 = time.perf_counter()
        fn()
        times.append(time.perf_counter() - t0)
    return statistics.median(times)


def cjpegli(src: Path, out: Path) -> None:
    subprocess.run(
        [str(s.CJPEGLI), str(src), str(out), "-q", str(JP_Q), "-p", "0", "--chroma_subsampling=444"],
        check=True,
        capture_output=True,
    )


def encoded_rows(path: Path) -> list[dict]:
    return [r for r in csv.DictReader(path.open(encoding="utf-8")) if not r.get("error") and not r.get("skip")]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("work", type=Path)
    ap.add_argument("--n", type=int, default=500, help="library sample size (camera: all rows)")
    ap.add_argument("--seed", type=int, default=2)
    a = ap.parse_args()
    work: Path = a.work

    tiny = s.write_ppm(s.pyvips.Image.black(8, 8, bands=3).cast("uchar"), work / "tiny.ppm")
    startup = median_time(lambda: cjpegli(tiny, work / "tiny.jpg"), 5)
    print(f"cjpegli process baseline (8x8): {startup * 1000:.1f} ms", flush=True)

    lib = encoded_rows(work / "library.csv")
    cam = encoded_rows(work / "camera.csv")
    lib = random.Random(a.seed).sample(lib, min(a.n, len(lib)))
    rows = []
    ppm = work / "time_in.ppm"
    for set_name, sample in (("library", lib), ("camera", cam)):
        for k, r in enumerate(sample, 1):
            px = load_px(Path(r["path"]))
            t_vips = median_time(lambda px=px: px.jpegsave_buffer(Q=75, subsample_mode="off", optimize_coding=True))
            s.write_ppm(px, ppm)
            t_jp = median_time(lambda: cjpegli(ppm, work / "time_out.jpg"))
            rows.append(
                {
                    "set": set_name,
                    "w": px.width,
                    "h": px.height,
                    "mpx": px.width * px.height / 1e6,
                    "vips_ms": t_vips * 1000,
                    "jp_ms": t_jp * 1000,
                    "jp_ms_net": max(t_jp - startup, 0) * 1000,
                }
            )
            if k % 50 == 0 or k == len(sample):
                print(f"{set_name} {k}/{len(sample)}", flush=True)

    with (work / "timing.csv").open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)
    for set_name in ("library", "camera"):
        rs = [r for r in rows if r["set"] == set_name]
        if not rs:
            continue
        per_mp = {k: statistics.median(float(r[k]) / float(r["mpx"]) for r in rs) for k in ("vips_ms", "jp_ms", "jp_ms_net")}
        ratio = sum(float(r["jp_ms"]) for r in rs) / sum(float(r["vips_ms"]) for r in rs)
        print(
            f"{set_name}: n={len(rs)} median ms/MP: vips Q75 {per_mp['vips_ms']:.1f}, cjpegli Q{JP_Q} "
            f"{per_mp['jp_ms']:.1f} (net of process start {per_mp['jp_ms_net']:.1f}), total time ratio {ratio:.2f}x",
            flush=True,
        )


if __name__ == "__main__":
    main()
