"""nvJPEG (torchvision.io.decode_jpeg on CUDA) against CPU decoders on Windows, for docs/PROFILING.md.

Run with the eval venv (torch with CUDA): python -I eval/jpeg-decoders/gpu.py FILE...

- GPU: batches of the same file (1 to 64, capped by the decoded size), per-image time with the pixels left on
  the GPU and with them copied back to the host (what CPU code after the decode needs), and the process's CPU
  time per image: nvJPEG's hybrid backend does the Huffman decoding on the CPU.
- CPU references in the same process: Pillow (libjpeg-turbo, as the app decodes) and pyvips (libvips), one
  thread, full size and with DCT scaling at the denominator `analyze` uses (1/2, 1/4, 1/8).
- Output: nvJPEG against Pillow, mean absolute difference per channel value and PSNR.
"""

import io
import os
import statistics
import sys
import time
from pathlib import Path

os.environ["VIPS_CONCURRENCY"] = "1"  # before pyvips loads libvips

import numpy as np
import pyvips
import torch
from PIL import Image
from torchvision.io import ImageReadMode, decode_jpeg

pyvips.cache_set_max(0)  # otherwise decoding the same buffer again is a cache hit
BUDGET_S = 1.5
MAX_BATCH_BYTES = 2.5e9  # decoded RGB per batch; the 2060 has 6 GB, ~1.4 GB in use by the desktop


def denom(w, h):
    """pg-core's choice for analyze: the smallest 1/8, 1/4, 1/2 keeping long ≥ 512 and short ≥ 320."""
    mw, mh = (512, 320) if w >= h else (320, 512)
    return next((d for d in (8, 4, 2) if -(-w // d) >= mw and -(-h // d) >= mh), 1)


def timed(fn, per_call=1):
    """Median seconds per item, and process CPU seconds per item, over at least BUDGET_S of calls."""
    for _ in range(3):
        fn()
    walls, cpu0, start, n = [], time.process_time(), time.perf_counter(), 0
    while len(walls) < 5 or time.perf_counter() - start < BUDGET_S:
        t = time.perf_counter()
        fn()
        walls.append((time.perf_counter() - t) / per_call)
        n += per_call
    return statistics.median(walls), (time.process_time() - cpu0) / n


def pillow(data, d=1):
    im = Image.open(io.BytesIO(data))
    if d > 1:
        im.draft("RGB", (-(-im.width // d), -(-im.height // d)))
    im.load()
    return im


def vips(data, d=1):
    return pyvips.Image.new_from_buffer(data, "", access="sequential", shrink=d).write_to_memory()


def main():
    assert torch.cuda.is_available(), "no CUDA device"
    vips_version = ".".join(str(pyvips.version(i)) for i in range(3))
    print(f"{torch.cuda.get_device_name()}, torch {torch.__version__}, Pillow {Image.__version__}, libvips {vips_version}")
    for path in sys.argv[1:]:
        data = Path(path).read_bytes()
        name = Path(path).stem
        im = pillow(data)
        w, h = im.size
        d = denom(w, h)
        ref = np.asarray(im.convert("RGB"), dtype=np.int16)
        cpu_tensor = torch.frombuffer(bytearray(data), dtype=torch.uint8)

        out = decode_jpeg(cpu_tensor, mode=ImageReadMode.RGB, device="cuda")
        torch.cuda.synchronize()
        gpu = out.permute(1, 2, 0).cpu().numpy().astype(np.int16)
        diff = np.abs(gpu - ref)
        mse = float((diff.astype(np.float64) ** 2).mean())
        psnr = float("inf") if mse == 0 else 10 * np.log10(255**2 / mse)

        print(f"\n{name}: {w}×{h} ({w * h / 1e6:.1f} MP), {len(data) / 1e3:.0f} KB")
        print(f"  nvJPEG vs Pillow: mean {diff.mean():.3f}, max {diff.max()}, {psnr:.1f} dB")
        for label, fn in [
            ("Pillow full", lambda data=data: pillow(data)),
            (f"Pillow 1/{d}", lambda data=data, d=d: pillow(data, d)),
            ("libvips full", lambda data=data: vips(data)),
            (f"libvips 1/{d}", lambda data=data, d=d: vips(data, d)),
        ]:
            wall, cpu = timed(fn)
            print(f"  {label:<22} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image")

        cap = max(1, int(MAX_BATCH_BYTES // (w * h * 3)))
        for batch in [b for b in (1, 8, 32, 64) if b <= cap] or [1]:
            items = [cpu_tensor] * batch

            def on_gpu(items=items):
                decode_jpeg(items, mode=ImageReadMode.RGB, device="cuda")
                torch.cuda.synchronize()

            def to_host(items=items):
                outs = decode_jpeg(items, mode=ImageReadMode.RGB, device="cuda")
                for o in outs:
                    o.cpu()
                torch.cuda.synchronize()

            for label, fn in (("on GPU", on_gpu), ("to host", to_host)):
                wall, cpu = timed(fn, batch)
                rate = f"{1 / wall:7.0f} img/s"
                print(f"  nvJPEG batch {batch:>2} {label:<8} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image   {rate}")
            torch.cuda.empty_cache()


if __name__ == "__main__":
    main()
