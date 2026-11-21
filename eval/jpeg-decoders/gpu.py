"""nvJPEG (torchvision.io.decode_jpeg on CUDA) against CPU decoders on Windows.

Run with the eval venv (torch with CUDA): python eval/jpeg-decoders/gpu.py FILE...

- GPU: batches of the same file (1 to 64, capped by the decoded size), per-image time with the pixels left on
  the GPU and with them copied back to the host (what CPU code after the decode needs), and the process's CPU
  time per image: nvJPEG's hybrid backend does the Huffman decoding on the CPU.
- CPU references in the same process: Pillow (libjpeg-turbo, as the app decodes) and pyvips (libvips), one
  thread, full size and with DCT scaling at the denominator `analyze` uses (1/2, 1/4, 1/8).
- Output: nvJPEG against Pillow, mean absolute difference per channel value and PSNR.
"""

import ctypes
import io
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))  # eval/: common's analysis rule
os.environ["VIPS_CONCURRENCY"] = "1"  # before pyvips loads libvips

import env
import numpy as np
import pyvips
import torch
from PIL import Image
from timing import timed_s
from torchvision.io import ImageReadMode, decode_jpeg

from common import analysis_denom

pyvips.cache_set_max(0)  # otherwise decoding the same buffer again is a cache hit
# Decoded RGB per GPU batch. Sized for the 6 GB card this bench runs on (the 2060, ~1.4 GB of it in use by the desktop); the
# other GPU benches cap with timing.py's MAX_BATCH_BYTES instead.
GPU_BENCH_BATCH_BYTES = 2.5e9
CU_CTX_SCHED_BLOCKING_SYNC = 0x04  # cudaDeviceScheduleBlockingSync: a synchronize() sleeps instead of spinning


def blocking_sync() -> None:
    """Make the primary context of device 0 block on synchronize(), so waiting for the GPU doesn't bill CPU time
    (spinning would count as the decode's CPU cost). Must run before the first CUDA call.
    Done through the driver's own DLL, so no torch is needed (the nvImageCodec venv has none)."""
    cuda = ctypes.CDLL("nvcuda.dll")
    if (rc := cuda.cuInit(0)) != 0:
        raise RuntimeError(f"cuInit: CUDA error (code {rc})")
    if (rc := cuda.cuDevicePrimaryCtxSetFlags(0, CU_CTX_SCHED_BLOCKING_SYNC)) != 0:
        raise RuntimeError(f"cuDevicePrimaryCtxSetFlags: CUDA error (code {rc}); the context may already be active")


def pillow(data, d=1):
    im = Image.open(io.BytesIO(data))
    if d > 1:
        w, h = im.size
        # Request the floor: Pillow picks the largest DCT scale that still gives at least the requested size, so this
        # keeps 1/d. The decoded size is the ceiling, ceil(w/d) (3840x2151 at d=4 is 960x538); a ceiling request can
        # pick a larger scale for a side that isn't a multiple of d.
        im.draft("RGB", (w // d, h // d))
        if im.size != (-(-w // d), -(-h // d)):
            raise RuntimeError(f"draft gave {im.size} for {w}x{h} at 1/{d}, expected {(-(-w // d), -(-h // d))}")
    im.load()
    return im


def vips(data, d=1):
    return pyvips.Image.new_from_buffer(data, "", access="sequential", shrink=d).write_to_memory()


def main():
    blocking_sync()
    assert torch.cuda.is_available(), "no CUDA device"
    vips_version = ".".join(str(pyvips.version(i)) for i in range(3))
    print(f"{torch.cuda.get_device_name()}, libvips {vips_version}")
    print(env.line())
    for path in sys.argv[1:]:
        data = Path(path).read_bytes()
        name = Path(path).stem
        im = pillow(data)
        w, h = im.size
        d = analysis_denom(w, h)
        ref = np.asarray(im.convert("RGB"), dtype=np.int16)
        cpu_tensor = torch.frombuffer(bytearray(data), dtype=torch.uint8)

        out = decode_jpeg(cpu_tensor, mode=ImageReadMode.RGB, device="cuda")
        torch.cuda.synchronize()
        gpu = out.permute(1, 2, 0).cpu().numpy().astype(np.int16)
        diff = np.abs(gpu - ref)
        mse = float((diff.astype(np.float64) ** 2).mean())
        psnr = float("inf") if mse == 0 else 10 * np.log10(255**2 / mse)

        print(f"\n{name}: {w}x{h} ({w * h / 1e6:.1f} MP), {len(data) / 1e3:.0f} KB")
        print(f"  nvJPEG vs Pillow: mean {diff.mean():.3f}, max {diff.max()}, {psnr:.1f} dB")
        for label, fn in [
            ("Pillow full", lambda data=data: pillow(data)),
            (f"Pillow 1/{d}", lambda data=data, d=d: pillow(data, d)),
            ("libvips full", lambda data=data: vips(data)),
            (f"libvips 1/{d}", lambda data=data, d=d: vips(data, d)),
        ]:
            wall, cpu = timed_s(fn)
            print(f"  {label:<22} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image")

        cap = max(1, int(GPU_BENCH_BATCH_BYTES // (w * h * 3)))
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
                wall, cpu = timed_s(fn, batch)
                rate = f"{1 / wall:7.0f} images/s"
                print(f"  nvJPEG batch {batch:>2} {label:<8} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image   {rate}")
            torch.cuda.empty_cache()


if __name__ == "__main__":
    main()
