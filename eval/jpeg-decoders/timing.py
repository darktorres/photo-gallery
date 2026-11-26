"""The timing the decoder harnesses share, in two methods that differ in cost, not in measurement:

- `timed_s`: a GPU decode, repeated over at least `budget` seconds (3 warm-ups, then at least 5 calls): the median wall
  time, and the process's CPU time per call.
- `median_s`: a call too slow to repeat for BUDGET_S (an encoder, a whole program run): a fixed number of calls, with no
  warm-up, over `seconds_of`, which is the one clock every harness reads.
- `blocking_sync`: the CUDA setup the GPU timings need, through the driver's DLL, so no torch is needed.
"""

import ctypes
import statistics
import time

# The seconds of calls one measurement takes, at least (the harnesses that time a GPU decode use it).
BUDGET_S = 1.5
# The decoded RGB one GPU batch may hold: the 2060 has 6 GB, ~1.5 GB of it in use by the desktop, and the decoders' own
# memory comes on top. The batched benches cap their batch sizes with max_batch.
MAX_BATCH_BYTES = 2.0e9
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


def primary_ctx_flags() -> int:
    """The flags of device 0's primary context, read through the driver's DLL (blocking sync is the 0x04 bit)."""
    cuda = ctypes.CDLL("nvcuda.dll")
    flags, active = ctypes.c_uint(), ctypes.c_int()
    if (rc := cuda.cuDevicePrimaryCtxGetState(0, ctypes.byref(flags), ctypes.byref(active))) != 0:
        raise RuntimeError(f"cuDevicePrimaryCtxGetState: CUDA error (code {rc})")
    return flags.value


def max_batch(w: int, h: int) -> int:
    """The largest batch of w x h RGB images within MAX_BATCH_BYTES, and at least 1."""
    return max(1, int(MAX_BATCH_BYTES // (w * h * 3)))


def timed_s(fn, per_call=1, budget=BUDGET_S):
    """Median seconds per item, and process CPU seconds per item, over at least `budget` seconds of calls."""
    for _ in range(3):
        fn()
    walls, cpu0, start, n = [], time.process_time(), time.perf_counter(), 0
    while len(walls) < 5 or time.perf_counter() - start < budget:
        t = time.perf_counter()
        fn()
        walls.append((time.perf_counter() - t) / per_call)
        n += per_call
    return statistics.median(walls), (time.process_time() - cpu0) / n


def seconds_of(fn):
    """Wall seconds of one call of `fn`."""
    t = time.perf_counter()
    fn()
    return time.perf_counter() - t


def median_s(reps, fn):
    """The median of `reps` calls of `fn`, each returning the seconds it took (`seconds_of`, or a run that times itself)."""
    return statistics.median(fn() for _ in range(reps))
