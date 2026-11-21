"""What the verifier's scripts share without the GPU stack: the rotation schedules, the CSV columns and the resumable
checkpoint. It imports no torch and no kornia, so verify.py and verify_synth.py take these from here.

Features (the local-feature extractors) and PairRunner (which drives align and compare) use torch and kornia, so they stay
in verify.py.
"""

import csv

from common import WORK_DIR

ROTATIONS = {"none": (), "90": (90, 180, 270), "45": (45, 90, 135, 180, 225, 270, 315)}

COLUMNS = [
    "id",
    "label",
    "similarity",
    "decision",
    "correct",
    "inliers",
    "matches",
    "flipped",
    "rotation",
    "reference",
    "cover_a",
    "cover_b",
    "overlap",
    "color_to",
    "noise",
    "res_p99",
    "changed_frac",
    "blob_frac",
    "interior_changed_frac",
    "interior_regions",
    "moved_frac",
    "moved_share",
    "shortcut",
    "error",
    "path_a",
    "path_b",
]


def csv_row(r: dict) -> dict:
    return {k: (f"{v:.4f}" if isinstance(v, float) else v) for k, v in r.items()}


class Checkpoint:
    """Appends every finished pair to <prefix>_<model>.partial.csv, so an interrupted run resumes where it
    stopped instead of starting over. Pair ids must be stable across runs (same inputs and options);
    delete the file to start fresh. Removed once the run completes."""

    def __init__(self, prefix: str, model: str) -> None:
        self.path = WORK_DIR / f"{prefix}_{model}.partial.csv"
        self.rows: dict[str, dict] = {}
        if self.path.exists():
            with self.path.open(newline="", encoding="utf-8") as f:
                self.rows = {r["id"]: {k: v for k, v in r.items() if v != ""} for r in csv.DictReader(f)}
            print(f"resuming: {len(self.rows)} pairs already done ({self.path.name})", flush=True)
        self._file = self.path.open("a", newline="", encoding="utf-8")  # kept open for the run
        self._writer = csv.DictWriter(self._file, COLUMNS, extrasaction="ignore")
        if not self.rows:
            self._writer.writeheader()

    def done(self, pid) -> dict | None:
        return self.rows.get(str(pid))

    def add(self, row: dict) -> None:
        self._writer.writerow(csv_row(row))
        self._file.flush()

    def finish(self) -> None:
        self._file.close()
        self.path.unlink()
