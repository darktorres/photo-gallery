"""HDD read-order benchmark: in which order should a library pass read the files?

Usage: python bench_hdd.py --root "<the folder>" [--stride 25 | --dense 7500]

Orders compared, each on its own disjoint sample, so no order benefits from another's cached reads:
every --stride-th file (a different offset per order: sparse, every read pays a seek), or with
--dense a block of that many consecutive files (a different block per order: close to a full pass).
  path         directory walk, names sorted (what a naive scanner does)
  mft          NTFS file reference number (the order an MFT/USN scan yields)
  lcn          first physical cluster on the volume (FSCTL_GET_RETRIEVAL_POINTERS)
  random       shuffled (the worst case)
  path_qd8     path order with 8 reads in flight (lets the drive's NCQ reorder)
  lcn_qd8      lcn order with 8 reads in flight

Reads are unbuffered (FILE_FLAG_NO_BUFFERING), so the Windows file cache never serves them, and
all metadata (stat, cluster map) is gathered before any timing. Read-only.
"""

import argparse
import ctypes
import json
import os
import random
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from ctypes import wintypes
from pathlib import Path

import numpy as np

from common import IMAGE_EXTS, WORK_DIR

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.CreateFileW.restype = wintypes.HANDLE
kernel32.CreateFileW.argtypes = [
    wintypes.LPCWSTR,
    wintypes.DWORD,
    wintypes.DWORD,
    wintypes.LPVOID,
    wintypes.DWORD,
    wintypes.DWORD,
    wintypes.HANDLE,
]
kernel32.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
kernel32.DeviceIoControl.argtypes = [
    wintypes.HANDLE,
    wintypes.DWORD,
    wintypes.LPVOID,
    wintypes.DWORD,
    wintypes.LPVOID,
    wintypes.DWORD,
    ctypes.POINTER(wintypes.DWORD),
    wintypes.LPVOID,
]
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.VirtualAlloc.restype = wintypes.LPVOID
kernel32.VirtualAlloc.argtypes = [wintypes.LPVOID, ctypes.c_size_t, wintypes.DWORD, wintypes.DWORD]

GENERIC_READ = 0x80000000
FILE_READ_ATTRIBUTES = 0x80
FILE_SHARE_READ_WRITE_DELETE = 0x7
OPEN_EXISTING = 3
FILE_FLAG_NO_BUFFERING = 0x20000000
FILE_FLAG_SEQUENTIAL_SCAN = 0x08000000
INVALID_HANDLE = wintypes.HANDLE(-1).value
FSCTL_GET_RETRIEVAL_POINTERS = 0x00090073
ERROR_MORE_DATA = 234
CHUNK = 1 << 20  # unbuffered reads must be sector multiples into sector-aligned memory


def open_file(path: str, access: int, flags: int = 0) -> int:
    h = kernel32.CreateFileW("\\\\?\\" + path, access, FILE_SHARE_READ_WRITE_DELETE, None, OPEN_EXISTING, flags, None)
    if h == INVALID_HANDLE:
        raise ctypes.WinError(ctypes.get_last_error())
    return h


def cluster_map(path: str) -> tuple[int, int]:
    """(first LCN, extent count) of a file; (-1, 0) for files stored inside the MFT record."""
    h = open_file(path, FILE_READ_ATTRIBUTES)
    try:
        start_vcn = ctypes.c_longlong(0)
        out = ctypes.create_string_buffer(64 * 1024)
        returned = wintypes.DWORD()
        ok = kernel32.DeviceIoControl(
            h, FSCTL_GET_RETRIEVAL_POINTERS, ctypes.byref(start_vcn), 8, out, len(out), ctypes.byref(returned), None
        )
        if not ok and ctypes.get_last_error() != ERROR_MORE_DATA:
            return -1, 0
        extents = int.from_bytes(out.raw[0:4], "little")
        if extents == 0:
            return -1, 0
        # RETRIEVAL_POINTERS_BUFFER: ExtentCount, pad, StartingVcn, then {NextVcn, Lcn} pairs.
        lcn = int.from_bytes(out.raw[24:32], "little", signed=True)
        return lcn, extents
    finally:
        kernel32.CloseHandle(h)


class Reader:
    """Reads whole files unbuffered into one aligned buffer per thread."""

    def __init__(self) -> None:
        self.buf = kernel32.VirtualAlloc(None, CHUNK, 0x3000, 0x04)  # MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE

    def read(self, path: str) -> int:
        h = open_file(path, GENERIC_READ, FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN)
        total = 0
        try:
            got = wintypes.DWORD()
            while True:
                if not kernel32.ReadFile(h, self.buf, CHUNK, ctypes.byref(got), None):
                    raise ctypes.WinError(ctypes.get_last_error())
                if got.value == 0:
                    return total
                total += got.value
        finally:
            kernel32.CloseHandle(h)


def walk(root: Path) -> list[str]:
    """All image files, in sorted directory-walk order."""
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        out += [str(Path(dirpath) / f) for f in sorted(filenames) if Path(f).suffix.lower() in IMAGE_EXTS]
    return out


def timed_read(paths: list[str], depth: int) -> tuple[float, int]:
    start = time.perf_counter()
    if depth == 1:
        reader = Reader()
        total = sum(reader.read(p) for p in paths)
    else:
        local = threading.local()

        def read(p: str) -> int:
            if not hasattr(local, "reader"):
                local.reader = Reader()
            return local.reader.read(p)

        with ThreadPoolExecutor(depth) as pool:
            total = sum(pool.map(read, paths))
    return time.perf_counter() - start, total


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, help="the folder to walk")
    parser.add_argument("--stride", type=int, default=25)
    parser.add_argument(
        "--dense", type=int, default=0, help="instead of every --stride-th file, read blocks of this many consecutive files"
    )
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()
    # The Win32 calls take a \\?\ path, which needs an absolute one: resolve the folder once, here.
    root = Path(args.root).resolve()

    t = time.perf_counter()
    files = walk(root)
    print(f"{len(files):,} files under {root} (walk {time.perf_counter() - t:.0f} s)", flush=True)

    orders = ["path", "mft", "lcn", "random", "path_qd8", "lcn_qd8"]
    if args.dense:
        # Contiguous blocks of the path order, spread over the folder: neighboring files, as in a full pass. The
        # blocks are disjoint only if each fits in one step of len(files) // len(orders).
        if args.dense * len(orders) > len(files):
            need = args.dense * len(orders)
            parser.error(f"--dense {args.dense:,} x {len(orders)} orders needs {need:,} files; {len(files):,} found")
        step = len(files) // len(orders)
        samples = {name: files[k * step : k * step + args.dense] for k, name in enumerate(orders)}
    else:
        samples = {name: files[k :: args.stride] for k, name in enumerate(orders)}
    all_sampled = [p for paths in samples.values() for p in paths]
    meta = {}
    t = time.perf_counter()
    for p in all_sampled:
        lcn, extents = cluster_map(p)
        meta[p] = (Path(p).stat().st_ino & 0xFFFFFFFFFFFF, lcn, extents)
    print(f"metadata for {len(meta):,} sampled files: {time.perf_counter() - t:.0f} s", flush=True)

    # How well does path order already follow the physical layout?
    position = {p: i for i, p in enumerate(files)}
    path_pos = np.array([position[p] for p in all_sampled], dtype=np.float64)
    mfts = np.array([meta[p][0] for p in all_sampled], dtype=np.float64)
    lcns = np.array([meta[p][1] for p in all_sampled], dtype=np.float64)
    extents = np.array([meta[p][2] for p in all_sampled])

    def spearman(x: np.ndarray, y: np.ndarray) -> float:
        rx, ry = np.argsort(np.argsort(x)), np.argsort(np.argsort(y))
        return float(np.corrcoef(rx, ry)[0, 1])

    layout = {
        "spearman_path_vs_lcn": spearman(path_pos, lcns),
        "spearman_mft_vs_lcn": spearman(mfts, lcns),
        "spearman_path_vs_mft": spearman(path_pos, mfts),
        "fragmented_files": float((extents > 1).mean()),
        "resident_files": float((extents == 0).mean()),
    }
    print("layout: " + ", ".join(f"{k} {v:.3f}" for k, v in layout.items()), flush=True)

    rng = random.Random(args.seed)
    rows = []
    for name in orders:
        paths = list(samples[name])
        base = name.split("_")[0]
        if base == "mft":
            paths.sort(key=lambda p: meta[p][0])
        elif base == "lcn":
            paths.sort(key=lambda p: meta[p][1])
        elif base == "random":
            rng.shuffle(paths)
        depth = 8 if name.endswith("qd8") else 1
        seconds, total = timed_read(paths, depth)
        row = {
            "order": name,
            "files": len(paths),
            "seconds": seconds,
            "files_s": len(paths) / seconds,
            "mb_s": total / seconds / 1e6,
            "avg_kb": total / len(paths) / 1e3,
        }
        rows.append(row)
        print(f"  {name}: {row['files_s']:.0f} files/s, {row['mb_s']:.0f} MB/s ({len(paths)} files, {seconds:.0f} s)", flush=True)

    out = {"root": args.root, "files": len(files), "stride": args.stride, "dense": args.dense, "layout": layout, "rows": rows}
    name = f"bench_hdd_dense{args.dense}.json" if args.dense else "bench_hdd.json"
    (WORK_DIR / name).write_text(json.dumps(out, indent=2), encoding="utf-8")
    print("\n| order | files/s | MB/s | days for 70M files at this rate |")
    print("|---|---|---|---|")
    for r in rows:
        print(f"| {r['order']} | {r['files_s']:.0f} | {r['mb_s']:.0f} | {70e6 / r['files_s'] / 86400:.1f} |")
    if not args.dense:
        print(
            f"\nNote: each sample is 1 in {args.stride} files, so seeks are longer than in a full pass; compare "
            "orders, not absolute rates."
        )


if __name__ == "__main__":
    main()
