"""Local viewer for the bake-off results.

Usage: python viewer.py [--port 8765] [--no-browser]  ->  http://127.0.0.1:8765

Shows every run directory under DATA_ROOT (PG_DATA_ROOT) that holds a files.txt (eval_smoke,
eval, ...): a model comparison, and the pair lists written by evaluate.py as
side-by-side thumbnails, plus verify.py's decisions and diagnostic images, and group.py's groups
(marking a group writes pair marks: same within a set, different across sets). Binds to localhost
only and serves only files inside DATA_ROOT. Thumbnails are cached under CACHE_DIR/viewer_thumbs.
"""

import argparse
import csv
import hashlib
import io
import json
import mimetypes
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
GROUPS_FILE = "groups_{}.json"  # group.py


def list_runs() -> list[dict]:
    runs = []
    for d in sorted(p for p in RUNS_ROOT.iterdir() if p.is_dir() and (p / "files.txt").exists()):
        models = sorted(p.stem.removeprefix("results_") for p in d.glob("results_*.json"))
        # Pair lists that exist only for some models (verify*.csv): kind -> models that have it.
        by_kind = {
            kind: [m for m in models if (d / pattern.format(m)).exists()]
            for kind, pattern in PAIR_FILES.items()
            if kind in DIAG_PREFIX
        }
        by_kind["groups"] = [m for m in models if (d / GROUPS_FILE.format(m)).exists()]
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
            with csv_path.open(newline="", encoding="utf-8") as f:
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
    with path.open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    for row in rows:
        row["size_a"] = image_size(row["path_a"])
        row["size_b"] = image_size(row["path_b"])
    return rows


def label_of(rel: str) -> str:
    """Where an image sits in the labeled layout: dups2/<NNN> (a labeled group), else its top folder."""
    parts = rel.split("/")
    return "/".join(parts[:2]) if len(parts) == 3 and parts[0] == "dups2" else parts[0] if len(parts) > 1 else ""


def groups(run: str, model: str) -> dict | None:
    """group.py's groups with each member's size and label folder."""
    if not re.fullmatch(r"[\w.-]+", model):
        raise ValueError("bad model")
    path = run_dir(run) / GROUPS_FILE.format(model)
    if not path.exists():
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    for g in data["groups"]:
        g["sizes"] = [image_size(p) for p in g["members"]]
        g["labels"] = [label_of(p) for p in g["members"]]
    return data


def diagnostic_file(run: str, model: str, kind: str, pair_id: str) -> Path:
    """verify.py's diagnostic image for one pair."""
    if not re.fullmatch(r"[\w.-]+", model) or kind not in DIAG_PREFIX or not pair_id.isdigit():
        raise ValueError("bad model, kind or id")
    path = run_dir(run) / f"{DIAG_PREFIX[kind]}_{model}" / f"{pair_id}.jpg"
    if not path.is_file():
        raise FileNotFoundError(path.name)
    return path


# Hand marks on verifier pairs: is it the same picture (incl. edits) or a different one? One file per
# run, keyed by the unordered pair of image paths, so marks survive reruns that renumber pair ids.
PAIR_MARKS = "pair_marks.json"
PAIR_MARK_VALUES = ("same", "different")
marks_lock = threading.Lock()


def pair_key(a: str, b: str) -> str:
    return "|".join(sorted((a, b)))


def _read_marks(run: str) -> dict[str, str]:
    """The run's marks file, read without the lock: only for a caller that holds marks_lock already."""
    path = run_dir(run) / PAIR_MARKS
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def load_marks(run: str) -> dict[str, str]:
    """The marks, read under marks_lock, so a reader never has the file open while set_marks replaces it (os.replace
    fails on Windows while a reader has the target open)."""
    with marks_lock:
        return _read_marks(run)


@lru_cache(maxsize=4)
def run_files(run: str, mtime: float) -> frozenset[str]:
    """The run's image list; mtime is part of the cache key, so an edited files.txt is reread."""
    del mtime
    return frozenset((run_dir(run) / "files.txt").read_text(encoding="utf-8").splitlines())


def set_mark(run: str, a: str, b: str, mark: str | None) -> dict:
    return set_marks(run, [(a, b, mark)])


def set_marks(run: str, items: list) -> dict:
    """Sets (mark) or clears (None) the marks of many pairs at once: [(a, b, mark), ...]."""
    known = run_files(run, (run_dir(run) / "files.txt").stat().st_mtime)
    for a, b, mark in items:
        if mark not in (None, *PAIR_MARK_VALUES):
            raise ValueError(f"bad mark: {mark}")
        if a not in known or b not in known or a == b:
            raise ValueError("unknown image path")
    target = run_dir(run) / PAIR_MARKS
    with marks_lock:
        marks = _read_marks(run)  # marks_lock is held here, and it is not reentrant: load_marks would deadlock
        for a, b, mark in items:
            if mark is None:
                marks.pop(pair_key(a, b), None)
            else:
                marks[pair_key(a, b)] = mark
        tmp = target.with_suffix(".tmp")
        tmp.write_text(json.dumps(marks, indent=1, ensure_ascii=False), encoding="utf-8")
        tmp.replace(target)
    return {"changed": len(items), "marked": len(marks)}


def dataset_file(rel: str) -> Path:
    path = (DATA_ROOT / rel).resolve()
    if not path.is_relative_to(DATA_ROOT.resolve()) or not path.is_file():
        raise FileNotFoundError(rel)
    return path


def thumbnail(path: Path, size: int) -> bytes:
    """JPEG thumbnail, upright, cached."""
    stat = path.stat()
    key = hashlib.sha1(f"{path}|{stat.st_mtime_ns}|{stat.st_size}|{size}".encode(), usedforsecurity=False).hexdigest()
    cached = THUMB_DIR / key[:2] / f"{key}.jpg"
    if cached.exists():
        return cached.read_bytes()
    im = load_rgb(path)
    im.thumbnail((size, size), Image.Resampling.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=85)
    data = buf.getvalue()
    cached.parent.mkdir(parents=True, exist_ok=True)
    tmp = cached.with_name(f"{key}.{threading.get_ident()}.tmp")
    tmp.write_bytes(data)
    tmp.replace(cached)
    return data


class Handler(BaseHTTPRequestHandler):
    def host_allowed(self) -> bool:
        # DNS rebinding: a page on another host name can resolve to 127.0.0.1, but its requests still carry that name.
        return self.headers.get("Host", "").rsplit(":", 1)[0] in ("127.0.0.1", "localhost")

    def do_GET(self) -> None:
        if not self.host_allowed():
            self.reply(b"forbidden", "text/plain", status=403)
            return
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
            elif url.path == "/api/groups":
                self.reply_json(groups(q["run"], q["model"]))
            elif url.path == "/img":
                path = dataset_file(q["path"])
                size = int(q.get("size", "0"))
                if size:
                    self.reply(thumbnail(path, size), "image/jpeg", cache=True)
                else:
                    mime = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
                    # The Windows registry maps .jpg to the non-standard image/jpg.
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
        except OSError as e:  # e.g. a file that can't be read right now: an answer, not a dropped connection
            self.reply(f"could not read: {e}".encode(), "text/plain", status=500)

    def do_POST(self) -> None:
        if not self.host_allowed():
            self.reply(b"forbidden", "text/plain", status=403)
            return
        url = urlparse(self.path)
        # JSON only: a cross-site page can't send application/json without a CORS
        # preflight, which this server never approves.
        if self.headers.get("Content-Type", "").split(";")[0].strip() != "application/json":
            self.reply(b"bad request: expected application/json", "text/plain", status=400)
            return
        try:
            body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
            if url.path == "/api/marks":
                self.reply_json(set_mark(body["run"], body["a"], body["b"], body.get("mark")))
            elif url.path == "/api/marks/bulk":
                self.reply_json(set_marks(body["run"], [(a, b, m) for a, b, m in body["marks"]]))
            else:
                self.reply(b"not found", "text/plain", status=404)
        except (ConnectionAbortedError, BrokenPipeError, ConnectionResetError):
            pass  # the browser went away mid-request, as in do_GET
        except (KeyError, ValueError, IndexError, TypeError) as e:
            self.reply(f"bad request: {e}".encode(), "text/plain", status=400)
        except OSError as e:  # e.g. PermissionError: the marks were not saved, and the page must say so
            self.reply(f"could not save the marks: {e}".encode(), "text/plain", status=500)

    def reply(self, body: bytes, content_type: str, status: int = 200, cache: bool = False) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "max-age=3600" if cache else "no-store")
        self.end_headers()
        self.wfile.write(body)

    def reply_json(self, obj) -> None:
        self.reply(json.dumps(obj).encode(), "application/json")

    def log_message(self, fmt: str, *args) -> None:
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
