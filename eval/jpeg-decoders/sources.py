#!/usr/bin/env python3
"""The source checkouts of the JPEG decoder comparison, for linux_build.py and win_build.py: each is fetched once at its
pinned commit, with the folder's patch (patches/<folder name>.patch) applied. win-build.cmd keeps its own copy until it is
removed (tools/README.md).
"""

import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent

# The pinned checkouts of linux_build.py and win_build.py, by folder name: (url, commit). Each pin is written here once.
# The rust-turbojpeg fork is the exception: its pin is tools/build_jpeg_turbo.py's FORK_URL and FORK_COMMIT. win-build.cmd
# keeps its own copies until it is removed.
PINS = {
    "mozjpeg": ("https://github.com/mozilla/mozjpeg", "08265790774cd0714832c9e675522acbe5581437"),
    "jpegli": ("https://github.com/google/jpegli", "031a0077f5799a6041004267fc12b956c1f52a20"),
    "guetzli": ("https://github.com/google/guetzli", "214f2bb42abf5a577c079d00add5d6cc470620d3"),
    "wuffs": ("https://github.com/google/wuffs", "d28ecd494491baa9b11d2557d10819111050fd24"),
    "parajpeg": ("https://github.com/anniigr/parajpeg", "f45081d7f75fdab809e7170c15fd806f42deb375"),
    "jpeggpu": ("https://github.com/nolmoonen/jpeggpu", "3ae7ac0dbc6e1b047abaa67badb6378baa7d39cb"),
    "GPUJPEG": ("https://github.com/CESNET/GPUJPEG", "37ab4810657fafeedad8c7a3c4546ea4ef5eb363"),
    "GPU-JPEG-Decoder": ("https://github.com/debesheedas/GPU-JPEG-Decoder", "61cfb3726691fe2436a84696d6a977d5bd8c6123"),
}


def clone(folder: Path, url: str, commit: str, submodules: bool = False) -> None:
    """Fetches `commit` of `url` into `folder`, once: a folder already there is kept as it is. LF line endings are kept
    (autocrlf off, as upstream). Prints the checkout's commit and date."""
    name = folder.name
    git = ["git", "-c", "core.autocrlf=false", "-C", str(folder)]
    if not folder.is_dir():
        folder.mkdir(parents=True)
        subprocess.run(["git", "init", "-q", str(folder)], check=True)
        subprocess.run([*git, "fetch", "-q", "--depth", "1", url, commit], check=True)
        subprocess.run([*git, "checkout", "-q", "FETCH_HEAD"], check=True)
        if submodules:
            subprocess.run([*git, "submodule", "update", "-q", "--init", "--recursive", "--depth", "1"], check=True)
        patch = HERE / "patches" / f"{name}.patch"
        if patch.is_file():
            subprocess.run([*git, "apply", str(patch)], check=True)
    head = subprocess.run([*git, "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run([*git, "log", "-1", "--format=%cs"], capture_output=True, text=True, check=True).stdout.strip()
    print(f"{name:<14} {head} {date}")
