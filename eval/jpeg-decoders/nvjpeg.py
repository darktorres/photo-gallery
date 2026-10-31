"""nvJPEG with Huffman decoding on the CPU (HYBRID) against on the GPU (GPU_HYBRID), for docs/PROFILING.md.

Run with the eval venv (torch with CUDA): python -I eval/jpeg-decoders/nvjpeg.py [--interpolate] [--batches 1,64] FILE...

torchvision's decode_jpeg has no backend option and decodes Huffman on the CPU, so this calls the nvJPEG DLL that
ships with torchvision directly (ctypes; declarations from nvjpeg.h 13.2). Device memory and streams come from torch.

- batched API (nvjpegDecodeBatched): GPU_HYBRID only decodes Huffman on the GPU for batches over 50 baseline
  interleaved JPEGs, so batches 1 to 128 (capped by the decoded size) show the switch.
- decoupled API (nvjpegDecodeJpegHost / TransferToDevice / Device), one image at a time: GPU_HYBRID uses GPU-assisted
  Huffman decoding for every image.
- each with the pixels left on the GPU and copied to a pinned host buffer (what CPU code after the decode needs),
  and the process's CPU time per image, which is what the import would get back.
- CPU reference in the same process: Pillow (libjpeg-turbo, as the app decodes), full size and with DCT scaling at
  the denominator `analyze` uses.
- output: each backend against Pillow (mean and max absolute difference), and GPU_HYBRID against HYBRID.
- --interpolate: NVJPEG_FLAGS_UPSAMPLING_WITH_INTERPOLATION, smooth chroma upsampling as libjpeg-turbo does (the
  default replicates chroma samples, ~29 dB from libjpeg-turbo on 4:2:0).
"""

import argparse
import ctypes as C
import io
import os
import statistics
import time
from pathlib import Path

import numpy as np
import torch
import torchvision
from PIL import Image

BUDGET_S = 1.5
MAX_BATCH_BYTES = 2.0e9  # decoded RGB per batch; the 2060 has 6 GB, ~1.5 GB in use by the desktop, plus nvJPEG's
BATCHES = (1, 8, 32, 64, 128)
HYBRID, GPU_HYBRID = 1, 2  # nvjpegBackend_t
BACKENDS = {"HYBRID (CPU Huffman)": HYBRID, "GPU_HYBRID (GPU Huffman)": GPU_HYBRID}
RGBI = 5  # nvjpegOutputFormat_t NVJPEG_OUTPUT_RGBI: interleaved RGB
UPSAMPLING_WITH_INTERPOLATION = 1 << 5  # NVJPEG_FLAGS_*


class Image_t(C.Structure):  # nvjpegImage_t
    _fields_ = [("channel", C.c_void_p * 4), ("pitch", C.c_size_t * 4)]  # noqa: RUF012 (ctypes reads it)


def load_nvjpeg():
    tv = Path(torchvision.__file__).parent
    os.add_dll_directory(str(Path(torch.__file__).parent / "lib"))  # cudart64_13.dll
    os.add_dll_directory(str(tv))
    lib = C.CDLL(str(tv / "nvjpeg64_13.dll"))
    v = C.c_void_p
    sigs = {
        "nvjpegCreateEx": [C.c_int, v, v, C.c_uint, C.POINTER(v)],
        "nvjpegDestroy": [v],
        "nvjpegJpegStateCreate": [v, C.POINTER(v)],
        "nvjpegJpegStateDestroy": [v],
        "nvjpegDecodeBatchedInitialize": [v, v, C.c_int, C.c_int, C.c_int],
        "nvjpegDecodeBatched": [v, v, C.POINTER(v), C.POINTER(C.c_size_t), C.POINTER(Image_t), v],
        "nvjpegDecoderCreate": [v, C.c_int, C.POINTER(v)],
        "nvjpegDecoderDestroy": [v],
        "nvjpegDecoderStateCreate": [v, v, C.POINTER(v)],
        "nvjpegDecoderJpegSupported": [v, v, v, C.POINTER(C.c_int)],
        "nvjpegBufferPinnedCreate": [v, v, C.POINTER(v)],
        "nvjpegBufferPinnedDestroy": [v],
        "nvjpegBufferDeviceCreate": [v, v, C.POINTER(v)],
        "nvjpegBufferDeviceDestroy": [v],
        "nvjpegStateAttachPinnedBuffer": [v, v],
        "nvjpegStateAttachDeviceBuffer": [v, v],
        "nvjpegJpegStreamCreate": [v, C.POINTER(v)],
        "nvjpegJpegStreamDestroy": [v],
        "nvjpegJpegStreamParse": [v, v, C.c_size_t, C.c_int, C.c_int, v],
        "nvjpegDecodeParamsCreate": [v, C.POINTER(v)],
        "nvjpegDecodeParamsDestroy": [v],
        "nvjpegDecodeParamsSetOutputFormat": [v, C.c_int],
        "nvjpegDecodeJpegHost": [v, v, v, v, v],
        "nvjpegDecodeJpegTransferToDevice": [v, v, v, v, v],
        "nvjpegDecodeJpegDevice": [v, v, v, C.POINTER(Image_t), v],
        "nvjpegGetProperty": [C.c_int, C.POINTER(C.c_int)],
    }
    for name, args in sigs.items():
        fn = getattr(lib, name)
        fn.argtypes, fn.restype = args, C.c_int
    return lib


NV = load_nvjpeg()


def blocking_sync():
    """Make waits on the GPU sleep instead of spin, so process CPU time counts work, not waiting.

    CUDA's default (cudaDeviceScheduleAuto) busy-waits in cudaStreamSynchronize, which would bill the whole GPU
    decode as CPU time. Must run before the CUDA context exists (torch creates it lazily).
    """
    cudart = C.CDLL(str(Path(torch.__file__).parent / "lib" / "cudart64_13.dll"))
    ok(cudart.cudaSetDeviceFlags(0x04), "cudaSetDeviceFlags(cudaDeviceScheduleBlockingSync)")
    return cudart


def ok(status, what):
    if status != 0:
        raise RuntimeError(f"{what}: status {status}")


CUDART = blocking_sync()


def new(fn, *args):
    out = C.c_void_p()
    ok(getattr(NV, fn)(*args, C.byref(out)), fn)
    return out


def nvjpeg_version():
    parts = []
    for prop in range(3):  # MAJOR_VERSION, MINOR_VERSION, PATCH_LEVEL
        val = C.c_int()
        ok(NV.nvjpegGetProperty(prop, C.byref(val)), "nvjpegGetProperty")
        parts.append(str(val.value))
    return ".".join(parts)


def image_desc(t):
    d = Image_t()
    d.channel[0], d.pitch[0] = t.data_ptr(), t.shape[1] * 3
    return d


class Batched:
    """nvjpegDecodeBatched on one handle and state, re-initialized per batch size."""

    def __init__(self, backend, data, w, h, batch, stream, flags=0):
        self.handle = new("nvjpegCreateEx", backend, None, None, flags)
        self.state = new("nvjpegJpegStateCreate", self.handle)
        ok(NV.nvjpegDecodeBatchedInitialize(self.handle, self.state, batch, 1, RGBI), "BatchedInitialize")
        self.src = (C.c_ubyte * len(data)).from_buffer_copy(data)
        self.ptrs = (C.c_void_p * batch)(*[C.addressof(self.src)] * batch)
        self.lens = (C.c_size_t * batch)(*[len(data)] * batch)
        self.outs = [torch.empty((h, w, 3), dtype=torch.uint8, device="cuda") for _ in range(batch)]
        self.dests = (Image_t * batch)(*[image_desc(t) for t in self.outs])
        self.stream = stream

    def decode(self):
        ok(
            NV.nvjpegDecodeBatched(self.handle, self.state, self.ptrs, self.lens, self.dests, self.stream.cuda_stream),
            "nvjpegDecodeBatched",
        )

    def close(self):
        NV.nvjpegJpegStateDestroy(self.state)
        NV.nvjpegDestroy(self.handle)


class Decoupled:
    """One image at a time through the decoupled API: host phase, transfer, device phase."""

    def __init__(self, backend, data, w, h, stream, flags=0):
        self.handle = new("nvjpegCreateEx", backend, None, None, flags)
        self.decoder = new("nvjpegDecoderCreate", self.handle, backend)
        self.state = new("nvjpegDecoderStateCreate", self.handle, self.decoder)
        self.pinned = new("nvjpegBufferPinnedCreate", self.handle, None)
        self.device = new("nvjpegBufferDeviceCreate", self.handle, None)
        ok(NV.nvjpegStateAttachPinnedBuffer(self.state, self.pinned), "AttachPinnedBuffer")
        ok(NV.nvjpegStateAttachDeviceBuffer(self.state, self.device), "AttachDeviceBuffer")
        self.jstream = new("nvjpegJpegStreamCreate", self.handle)
        self.params = new("nvjpegDecodeParamsCreate", self.handle)
        ok(NV.nvjpegDecodeParamsSetOutputFormat(self.params, RGBI), "SetOutputFormat")
        self.src = (C.c_ubyte * len(data)).from_buffer_copy(data)
        self.out = torch.empty((h, w, 3), dtype=torch.uint8, device="cuda")
        self.dest = image_desc(self.out)
        self.stream = stream
        self.parse()
        flag = C.c_int(-1)
        ok(NV.nvjpegDecoderJpegSupported(self.decoder, self.jstream, self.params, C.byref(flag)), "JpegSupported")
        self.supported = flag.value == 0  # nvjpeg.h: 0 means the decoder can handle the stream

    def parse(self):
        ok(NV.nvjpegJpegStreamParse(self.handle, self.src, len(self.src), 0, 0, self.jstream), "JpegStreamParse")

    def decode(self):
        s = self.stream.cuda_stream
        self.parse()
        ok(NV.nvjpegDecodeJpegHost(self.handle, self.decoder, self.state, self.params, self.jstream), "DecodeJpegHost")
        ok(NV.nvjpegDecodeJpegTransferToDevice(self.handle, self.decoder, self.state, self.jstream, s), "Transfer")
        ok(NV.nvjpegDecodeJpegDevice(self.handle, self.decoder, self.state, C.byref(self.dest), s), "DecodeJpegDevice")

    @property
    def outs(self):
        return [self.out]

    def close(self):
        NV.nvjpegDecodeParamsDestroy(self.params)
        NV.nvjpegJpegStreamDestroy(self.jstream)
        NV.nvjpegJpegStateDestroy(self.state)
        NV.nvjpegBufferDeviceDestroy(self.device)
        NV.nvjpegBufferPinnedDestroy(self.pinned)
        NV.nvjpegDecoderDestroy(self.decoder)
        NV.nvjpegDestroy(self.handle)


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
    if d > 1:  # draft() keeps a scale only if it computes (floor) at least the requested size
        im.draft("RGB", (im.width // d, im.height // d))
    im.load()
    return im


def report(label, dec, batch, host):
    """Time dec.decode() with the pixels left on the GPU and copied to pinned host buffers."""

    def on_gpu():
        dec.decode()
        dec.stream.synchronize()

    def to_host():
        dec.decode()
        with torch.cuda.stream(dec.stream):
            for o, hbuf in zip(dec.outs, host, strict=True):
                hbuf.copy_(o, non_blocking=True)
        dec.stream.synchronize()

    for mode, fn in (("on GPU", on_gpu), ("to host", to_host)):
        wall, cpu = timed(fn, batch)
        print(f"  {label:<40} {mode:<8} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image   {1 / wall:6.0f} img/s")


def diff(a, b):
    d = np.abs(a.astype(np.int16) - b.astype(np.int16))
    return f"mean {d.mean():.3f}, max {d.max()}"


def main():
    ap = argparse.ArgumentParser(description="nvJPEG HYBRID against GPU_HYBRID (see the module docstring)")
    ap.add_argument("--interpolate", action="store_true", help="smooth chroma upsampling")
    ap.add_argument("--batches", default=",".join(map(str, BATCHES)), help="comma-separated batch sizes")
    ap.add_argument("files", nargs="+")
    args = ap.parse_args()
    hflags = UPSAMPLING_WITH_INTERPOLATION if args.interpolate else 0
    batches = [int(b) for b in args.batches.split(",")]
    assert torch.cuda.is_available(), "no CUDA device"
    print(
        f"{torch.cuda.get_device_name()}, torch {torch.__version__}, nvJPEG {nvjpeg_version()} (torchvision's DLL),"
        f" Pillow {Image.__version__}, chroma upsampling {'interpolated' if args.interpolate else 'default'}"
    )
    stream = torch.cuda.Stream()  # creates the context
    flags = C.c_uint()
    ok(CUDART.cudaGetDeviceFlags(C.byref(flags)), "cudaGetDeviceFlags")
    assert flags.value & 0x07 == 0x04, f"context not in blocking sync: flags {flags.value:#x}"
    for path in args.files:
        data = Path(path).read_bytes()
        im = pillow(data)
        w, h = im.size
        d = denom(w, h)
        ref = np.asarray(im.convert("RGB"))
        print(f"\n{Path(path).stem}: {w}×{h} ({w * h / 1e6:.1f} MP), {len(data) / 1e3:.0f} KB")
        sw, sh = pillow(data, d).size
        for label, fn in (
            ("Pillow full", lambda data=data: pillow(data)),
            (f"Pillow 1/{d} ({sw}×{sh})", lambda data=data, d=d: pillow(data, d)),
        ):
            wall, cpu = timed(fn)
            print(f"  {label:<49} {wall * 1e3:8.2f} ms/image   CPU {cpu * 1e3:7.2f} ms/image   {1 / wall:6.0f} img/s")

        outputs = {}
        for name, backend in BACKENDS.items():
            dec = Decoupled(backend, data, w, h, stream, hflags)
            try:
                host = [torch.empty((h, w, 3), dtype=torch.uint8).pin_memory()]
                dec.decode()
                stream.synchronize()
                outputs[name] = dec.out.cpu().numpy()
                note = "" if dec.supported else " (decoder reports the stream unsupported)"
                print(f"  {name} vs Pillow: {diff(outputs[name], ref)}{note}")
                report(f"{name}, decoupled", dec, 1, host)
            except RuntimeError as e:
                print(f"  {name}, decoupled: {e}")
            finally:
                dec.close()
                torch.cuda.empty_cache()
        if len(outputs) == 2:
            print(f"  GPU_HYBRID vs HYBRID: {diff(*outputs.values())}")

        cap = max(1, int(MAX_BATCH_BYTES // (w * h * 3)))
        for batch in [b for b in batches if b <= cap]:
            host = [torch.empty((h, w, 3), dtype=torch.uint8).pin_memory() for _ in range(batch)]
            for name, backend in BACKENDS.items():
                try:
                    dec = Batched(backend, data, w, h, batch, stream, hflags)
                except RuntimeError as e:
                    print(f"  {name}, batch {batch}: {e}")
                    continue
                try:
                    report(f"{name}, batch {batch}", dec, batch, host)
                except RuntimeError as e:
                    print(f"  {name}, batch {batch}: {e}")
                finally:
                    dec.close()
                    del dec
                    torch.cuda.empty_cache()
            del host
        skipped = [b for b in batches if b > cap]
        if skipped:
            print(f"  batches {skipped} skipped: over {MAX_BATCH_BYTES / 1e9:.1f} GB of decoded RGB")


if __name__ == "__main__":
    main()
