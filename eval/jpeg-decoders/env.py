"""The package versions that produced a number, printed at the top of each script's output (docs/CODE_REVIEW.md,
decision 5). A script run from its venv imports this module beside it (the scripts run without `python -I`, so the
script's folder is on `sys.path`). Versions are read from the installed metadata, so nothing heavy is imported here:
the nvImageCodec venv has no torch, and it prints what it has.
"""

import contextlib
from importlib import metadata

# (distribution name, the label printed)
PACKAGES = (
    ("numpy", "numpy"),
    ("pillow", "Pillow"),
    ("torch", "torch"),
    ("nvidia-nvimgcodec-cu13", "nvImageCodec"),
)


def line() -> str:
    """One line: `numpy 2.5.4, Pillow 13.0.0, ...`, for each package installed in this venv."""
    found = []
    for dist, label in PACKAGES:
        with contextlib.suppress(metadata.PackageNotFoundError):
            found.append(f"{label} {metadata.version(dist)}")
    return ", ".join(found)
