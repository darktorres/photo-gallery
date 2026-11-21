"""The timing the decoder harnesses share, in two methods that differ in cost, not in measurement:

- `timed`: a GPU decode, repeated over at least BUDGET_S (3 warm-ups, then at least 5 calls): the median wall time, and
  the process's CPU time per call.
- `median_s`: a call too slow to repeat for BUDGET_S (an encoder, a whole program run): a fixed number of calls, with no
  warm-up, over `seconds_of`, which is the one clock every harness reads.
"""

import statistics
import time

# The seconds of calls one measurement takes, at least (the harnesses that time a GPU decode use it).
BUDGET_S = 1.5
# The decoded RGB one GPU batch may hold: the 2060 has 6 GB, ~1.5 GB of it in use by the desktop, and the decoders' own
# memory comes on top. The batched benches cap their batch sizes with it.
MAX_BATCH_BYTES = 2.0e9


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


def seconds_of(fn):
    """Wall seconds of one call of `fn`."""
    t = time.perf_counter()
    fn()
    return time.perf_counter() - t


def median_s(reps, fn):
    """The median of `reps` calls of `fn`, each returning the seconds it took (`seconds_of`, or a run that times itself)."""
    return statistics.median(fn() for _ in range(reps))
