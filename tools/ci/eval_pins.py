#!/usr/bin/env python3
"""The packages of eval/requirements.txt that CI's eval tests import, at their pins: pillow (the viewer imports it) and
numpy (the bench tests import it). `install` installs them into this Python. The rest of that file is the GPU venv (torch,
TensorRT, two extra indexes), which CI does not install.

    python tools/ci/eval_pins.py install
"""

import re
import subprocess
import sys
from pathlib import Path

REQUIREMENTS = Path(__file__).resolve().parents[2] / "eval" / "requirements.txt"
# The packages CI installs for the eval tests, each pinned once in REQUIREMENTS.
PACKAGES = ("pillow", "numpy")


def pin(text: str, package: str) -> str | None:
    """The `<package>==<version>` line of a requirements file, or None when it has none."""
    match = re.search(rf"^{re.escape(package)}==\S+", text, re.IGNORECASE | re.MULTILINE)
    return match.group(0) if match else None


def main() -> None:
    if sys.argv[1:] != ["install"]:
        sys.exit("usage: eval_pins.py install")
    text = REQUIREMENTS.read_text(encoding="utf-8")
    requirements = []
    for package in PACKAGES:
        requirement = pin(text, package)
        if requirement is None:
            sys.exit(f"{REQUIREMENTS} has no {package} pin")
        requirements.append(requirement)
    result = subprocess.run([sys.executable, "-m", "pip", "install", "--quiet", *requirements], check=False)
    if result.returncode != 0:
        sys.exit(result.returncode)


if __name__ == "__main__":
    main()
