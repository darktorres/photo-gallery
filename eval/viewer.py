"""Local viewer for the bake-off results.

Usage: python viewer.py [--port 8765] [--no-browser]  ->  http://127.0.0.1:8765

Shows every run directory under D:\\PhotoGallery that holds a files.txt (eval_smoke,
eval, ...): a model comparison, and the pair lists written by evaluate.py as
side-by-side thumbnails, plus verify.py's decisions and diagnostic images. The Borders page
reviews tools/borders.py scans (any folder under D:\\PhotoGallery with a report.csv) and saves
false-positive marks to that folder's review.json. Binds to localhost only and serves only files inside
DATA_ROOT. Thumbnails are cached under CACHE_DIR/viewer_thumbs.
"""

import argparse
import csv
import hashlib
import io
import json
import mimetypes
import os
import re
import socket
import threading
import webbrowser
from functools import lru_cache
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from PIL import Image

from common import CACHE_DIR, DATA_ROOT, WORK_DIR, load_rgb

RUNS_ROOT = WORK_DIR.parent
THUMB_DIR = CACHE_DIR / "viewer_thumbs"
PAGE = Path(__file__).with_name("viewer.html")
PAIR_FILES = {
    "misses": "misses_{}.csv",
    "hard_negatives": "hard_negatives_{}.csv",
    "review": "review_{}.csv",
    "unpaired": "unpaired_pairs_{}.csv",
    "verify": "verify_{}.csv",
    "verify_cross": "verify_cross_{}.csv",
    "verify_knn": "verify_knn_{}.csv",
}
# verify.py's diagnostic image folders, by pair kind.
DIAG_PREFIX = {"verify": "verify", "verify_cross": "verify_cross", "verify_knn": "verify_knn"}

# tools/borders.py output folders (under RUNS_ROOT, holding report.csv) and the review verdicts
# saved next to it; borders.py --apply skips false positives. Keep in sync with borders.py.
BORDER_REVIEW = "review.json"
FALSE_POSITIVE = "false_positive"
review_lock = threading.Lock()


def list_runs() -> list[dict]:
    runs = []
    for d in sorted(p for p in RUNS_ROOT.iterdir() if p.is_dir() and (p / "files.txt").exists()):
        models = sorted(p.stem.removeprefix("results_") for p in d.glob("results_*.json"))
        # Pair lists that exist only for some models (verify*.csv): kind -> models that have it.
        by_kind = {kind: [m for m in models if (d / pattern.format(m)).exists()]
                   for kind, pattern in PAIR_FILES.items() if kind in DIAG_PREFIX}
        runs.append({"name": d.name, "models": models, "models_by_kind": by_kind})
    return runs


def run_dir(name: str) -> Path:
    d = (RUNS_ROOT / name).resolve()
    if d.parent != RUNS_ROOT.resolve() or not (d / "files.txt").exists():
        raise ValueError(f"unknown run: {name}")
    return d


def summary(run: str) -> dict:
    d = run_dir(run)
    out = {}
    for results in sorted(d.glob("results_*.json")):
        model = results.stem.removeprefix("results_")
        synth = d / f"synth_{model}.json"
        out[model] = {
            "results": json.loads(results.read_text(encoding="utf-8")),
            "synth": json.loads(synth.read_text(encoding="utf-8")) if synth.exists() else None,
        }
    return out


def benchmarks(run: str) -> dict:
    """Speed benchmarks (bench_decode.py, bench_hdd.py) and verify.py confusion tables of one run."""
    d = run_dir(run)

    def load(path: Path) -> dict:
        return json.loads(path.read_text(encoding="utf-8"))

    decode = d / "bench_decode.json"
    marks = load_marks(run)
    verify = {}
    # verify_<model>.json, verify_cross_<model>.json, verify_knn_<model>.json
    for p in sorted(d.glob("verify*.json")):
        v = load(p)
        # Hand marks per label: how many pairs are marked, and how often the verifier agrees.
        v["marks"] = {}
        csv_path = p.with_suffix(".csv")
        if csv_path.exists():
            with open(csv_path, newline="", encoding="utf-8") as f:
                for r in csv.DictReader(f):
                    mark = marks.get(pair_key(r["path_a"], r["path_b"]))
                    if not mark:
                        continue
                    m = v["marks"].setdefault(r["label"], {"marked": 0, "agree": 0})
                    m["marked"] += 1
                    m["agree"] += {"match": "same", "unrelated": "different"}.get(r["decision"]) == mark
        verify[p.stem] = v
    return {
        "decode": load(decode) if decode.exists() else None,
        "hdd": [load(p) for p in sorted(d.glob("bench_hdd*.json"))],
        "verify": verify,
    }


def image_size(rel: str) -> str:
    try:
        with Image.open(DATA_ROOT / rel) as im:  # reads the header only
            return f"{im.width}×{im.height}"
    except OSError:
        return "?"


def pairs(run: str, model: str, kind: str) -> list[dict]:
    if not re.fullmatch(r"[\w.-]+", model) or kind not in PAIR_FILES:
        raise ValueError("bad model or kind")
    path = run_dir(run) / PAIR_FILES[kind].format(model)
    if not path.exists():
        return []
    with open(path, newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    for row in rows:
        row["size_a"] = image_size(row["path_a"])
        row["size_b"] = image_size(row["path_b"])
    return rows


def diagnostic_file(run: str, model: str, kind: str, pair_id: str) -> Path:
    """verify.py's diagnostic image for one pair."""
    if not re.fullmatch(r"[\w.-]+", model) or kind not in DIAG_PREFIX or not pair_id.isdigit():
        raise ValueError("bad model, kind or id")
    path = run_dir(run) / f"{DIAG_PREFIX[kind]}_{model}" / f"{pair_id}.jpg"
    if not path.is_file():
        raise FileNotFoundError(path.name)
    return path


def border_scans() -> list[str]:
    return sorted(p.name for p in RUNS_ROOT.iterdir() if p.is_dir() and (p / "report.csv").exists())


def border_dir(scan: str) -> Path:
    d = (RUNS_ROOT / scan).resolve()
    if d.parent != RUNS_ROOT.resolve() or not (d / "report.csv").exists():
        raise ValueError(f"unknown border scan: {scan}")
    return d


def side_kind(side: str) -> str:
    """Side.describe() text -> black / white / color / blur (same buckets as report.md)."""
    words = side.split()
    if words[1] == "blur":
        return "blur"
    rgb = [int(words[2][i:i + 2], 16) for i in (1, 3, 5)]
    return "black" if max(rgb) <= 40 else "white" if min(rgb) >= 215 else "color"


_border_cache: dict[Path, tuple[int, list[dict]]] = {}


def border_rows(scan: str) -> list[dict]:
    """The scan's detections (status "border"), cached until report.csv changes."""
    report = border_dir(scan) / "report.csv"
    mtime = report.stat().st_mtime_ns
    cached = _border_cache.get(report)
    if cached and cached[0] == mtime:
        return cached[1]
    with open(report, newline="", encoding="utf-8") as f:
        rows = [r for r in csv.DictReader(f) if r["status"] == "border"]
    out = []
    for i, r in enumerate(rows):
        out.append({
            "i": i, "path": r["path"], "name": Path(r["path"]).name, "folder": Path(r["path"]).parent.name,
            "width": int(r["width"]), "height": int(r["height"]),
            "crop": [int(r[k]) for k in ("crop_left", "crop_top", "crop_right", "crop_bottom")],
            "sides": r["sides"], "kinds": sorted({side_kind(s) for s in r["sides"].split("; ")}),
        })
    _border_cache[report] = (mtime, out)
    return out


def load_review(scan: str) -> dict[str, str]:
    path = border_dir(scan) / BORDER_REVIEW
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def borders(scan: str) -> dict:
    review = load_review(scan)
    return {"rows": [{**r, "verdict": review.get(r["path"])} for r in border_rows(scan)]}


def set_verdict(scan: str, i: int, verdict: str | None) -> dict:
    if verdict not in (None, FALSE_POSITIVE):
        raise ValueError(f"bad verdict: {verdict}")
    path = border_rows(scan)[i]["path"]
    target = border_dir(scan) / BORDER_REVIEW
    with review_lock:
        review = load_review(scan)
        if verdict is None:
            review.pop(path, None)
        else:
            review[path] = verdict
        tmp = target.with_suffix(".tmp")
        tmp.write_text(json.dumps(review, indent=1, ensure_ascii=False), encoding="utf-8")
        os.replace(tmp, target)
    return {"verdict": verdict, "false_positives": sum(v == FALSE_POSITIVE for v in review.values())}


# Hand marks on verifier pairs: is it the same picture (incl. edits) or a different one? One file per
# run, keyed by the unordered pair of image paths, so marks survive reruns that renumber pair ids.
PAIR_MARKS = "pair_marks.json"
PAIR_MARK_VALUES = ("same", "different")


def pair_key(a: str, b: str) -> str:
    return "|".join(sorted((a, b)))


def load_marks(run: str) -> dict[str, str]:
    path = run_dir(run) / PAIR_MARKS
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


@lru_cache(maxsize=4)
def run_files(run: str, mtime: float) -> frozenset[str]:
    """The run's image list; mtime is part of the cache key, so an edited files.txt is reread."""
    del mtime
    return frozenset((run_dir(run) / "files.txt").read_text(encoding="utf-8").splitlines())


def set_mark(run: str, a: str, b: str, mark: str | None) -> dict:
    if mark not in (None, *PAIR_MARK_VALUES):
        raise ValueError(f"bad mark: {mark}")
    known = run_files(run, (run_dir(run) / "files.txt").stat().st_mtime)
    if a not in known or b not in known:
        raise ValueError("unknown image path")
    target = run_dir(run) / PAIR_MARKS
    with review_lock:
        marks = load_marks(run)
        if mark is None:
            marks.pop(pair_key(a, b), None)
        else:
            marks[pair_key(a, b)] = mark
        tmp = target.with_suffix(".tmp")
        tmp.write_text(json.dumps(marks, indent=1, ensure_ascii=False), encoding="utf-8")
        os.replace(tmp, target)
    return {"mark": mark, "marked": len(marks)}


def dataset_file(rel: str) -> Path:
    path = (DATA_ROOT / rel).resolve()
    if not path.is_relative_to(DATA_ROOT.resolve()) or not path.is_file():
        raise FileNotFoundError(rel)
    return path


def thumbnail(path: Path, size: int, upright: bool = True) -> bytes:
    """JPEG thumbnail, cached. upright=False keeps the stored orientation (ignores EXIF rotation),
    for drawing coordinates measured on the stored pixels, like border crops."""
    stat = path.stat()
    key = hashlib.sha1(f"{path}|{stat.st_mtime_ns}|{stat.st_size}|{size}|{upright}".encode()).hexdigest()
    cached = THUMB_DIR / key[:2] / f"{key}.jpg"
    if cached.exists():
        return cached.read_bytes()
    if upright:
        im = load_rgb(path)
    else:
        with Image.open(path) as raw:
            im = raw.convert("RGB")
    im.thumbnail((size, size), Image.Resampling.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=85)
    data = buf.getvalue()
    cached.parent.mkdir(parents=True, exist_ok=True)
    tmp = cached.with_name(f"{key}.{threading.get_ident()}.tmp")
    tmp.write_bytes(data)
    os.replace(tmp, cached)
    return data


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        url = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        try:
            if url.path == "/":
                self.reply(PAGE.read_bytes(), "text/html; charset=utf-8")
            elif url.path == "/api/runs":
                self.reply_json(list_runs())
            elif url.path == "/api/summary":
                self.reply_json(summary(q["run"]))
            elif url.path == "/api/benchmarks":
                self.reply_json(benchmarks(q["run"]))
            elif url.path == "/api/marks":
                self.reply_json(load_marks(q["run"]))
            elif url.path == "/api/pairs":
                self.reply_json(pairs(q["run"], q["model"], q["kind"]))
            elif url.path == "/img":
                path = dataset_file(q["path"])
                size = int(q.get("size", "0"))
                if size:
                    self.reply(thumbnail(path, size), "image/jpeg", cache=True)
                else:
                    mime = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
                    # The Windows registry maps .jpg to the non-standard image/jpg.
                    self.reply(path.read_bytes(), "image/jpeg" if mime == "image/jpg" else mime, cache=True)
            elif url.path == "/api/borders/scans":
                self.reply_json(border_scans())
            elif url.path == "/api/borders":
                self.reply_json(borders(q["scan"]))
            elif url.path == "/border-img":
                # By row index, so only files listed in the scan's report can be served.
                path = Path(border_rows(q["scan"])[int(q["i"])]["path"])
                size = int(q.get("size", "0"))
                if size:
                    self.reply(thumbnail(path, size, upright=False), "image/jpeg", cache=True)
                else:
                    mime = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
                    self.reply(path.read_bytes(), "image/jpeg" if mime == "image/jpg" else mime, cache=True)
            elif url.path == "/diag":
                # Not cached: re-running verify.py rewrites these under the same names.
                self.reply(diagnostic_file(q["run"], q["model"], q["kind"], q["id"]).read_bytes(), "image/jpeg")
            else:
                self.reply(b"not found", "text/plain", status=404)
        except (KeyError, ValueError, IndexError, FileNotFoundError) as e:
            self.reply(f"bad request: {e}".encode(), "text/plain", status=400)
        except (ConnectionAbortedError, BrokenPipeError):
            pass  # the browser cancelled (e.g. scrolled past a lazy image)

    def do_POST(self) -> None:
        url = urlparse(self.path)
        try:
            # JSON only: a cross-site page can't send application/json without a CORS
            # preflight, which this server never approves.
            if self.headers.get("Content-Type", "").split(";")[0].strip() != "application/json":
                raise ValueError("expected application/json")
            body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
            if url.path == "/api/borders/review":
                self.reply_json(set_verdict(body["scan"], int(body["i"]), body.get("verdict")))
            elif url.path == "/api/marks":
                self.reply_json(set_mark(body["run"], body["a"], body["b"], body.get("mark")))
            else:
                self.reply(b"not found", "text/plain", status=404)
        except (KeyError, ValueError, IndexError, TypeError) as e:
            self.reply(f"bad request: {e}".encode(), "text/plain", status=400)

    def reply(self, body: bytes, content_type: str, status: int = 200, cache: bool = False) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "max-age=3600" if cache else "no-store")
        self.end_headers()
        self.wfile.write(body)

    def reply_json(self, obj) -> None:
        self.reply(json.dumps(obj).encode(), "application/json")

    def log_message(self, format: str, *args) -> None:
        pass


class Server(ThreadingHTTPServer):
    # The stdlib default sets SO_REUSEADDR, which on Windows lets this server bind a port
    # another server already listens on, and then silently lose requests to it. Claim
    # the port exclusively instead, so an occupied port fails loudly.
    allow_reuse_address = False
    daemon_threads = True

    def server_bind(self) -> None:
        if hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        super().server_bind()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8765, help="first port to try; the next free one is used")
    parser.add_argument("--no-browser", action="store_true")
    args = parser.parse_args()

    for port in range(args.port, args.port + 20):
        try:
            server = Server(("127.0.0.1", port), Handler)
            break
        except OSError:
            continue
    else:
        raise SystemExit(f"no free port in {args.port}-{args.port + 19}")
    url = f"http://127.0.0.1:{port}"
    print(f"viewer: {url}  (runs from {RUNS_ROOT}, images from {DATA_ROOT})", flush=True)
    if not args.no_browser:
        webbrowser.open(url)
    server.serve_forever()


if __name__ == "__main__":
    main()
