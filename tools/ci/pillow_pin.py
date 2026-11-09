#!/usr/bin/env python3
"""The Pillow pin of eval/requirements.txt, the one place it is written. `install` installs that pin into this Python: CI's
eval tests import the viewer, which imports Pillow. The rest of that file is the GPU venv (torch, TensorRT, two extra
indexes), which CI does not install.

    python tools/ci/pillow_pin.py install
"""

import re
import subprocess
import sys
from pathlib import Path

REQUIREMENTS = Path(__file__).resolve().parents[2] / "eval" / "requirements.txt"


def pin(text: str) -> str | None:
    """The `pillow==<version>` line of a requirements file, or None when it has none."""
    match = re.search(r"^pillow==\S+", text, re.IGNORECASE | re.MULTILINE)
    return match.group(0) if match else None


def main() -> None:
    if sys.argv[1:] != ["install"]:
        sys.exit("usage: pillow_pin.py install")
    requirement = pin(REQUIREMENTS.read_text(encoding="utf-8"))
    if requirement is None:
        sys.exit(f"{REQUIREMENTS} has no pillow pin")
    result = subprocess.run([sys.executable, "-m", "pip", "install", "--quiet", requirement], check=False)
    if result.returncode != 0:
        sys.exit(result.returncode)


if __name__ == "__main__":
    main()
