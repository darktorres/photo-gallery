#!/usr/bin/env python3
"""The source checkouts of the JPEG decoder comparison, for linux_build.py and win_build.py: each is fetched once at its
pinned commit, with the folder's patch (patches/<folder name>.patch) applied. The batch build it once shared pins with,
win-build.cmd, is removed (tools/README.md).
"""

import os
import shutil
import stat
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent

# The pinned checkouts of linux_build.py and win_build.py, by folder name: (url, commit). Each pin is written here once.
# The rust-turbojpeg fork is the exception: its pin is tools/build_jpeg_turbo.py's FORK_URL and FORK_COMMIT.
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
    if not folder.is_dir():
        # The checkout is made in a staging folder and renamed into place only when it is whole: a fetch or patch that
        # fails leaves nothing at `folder`, so the next run fetches again instead of keeping a half-made checkout.
        staging = folder.with_name(f"{name}.partial")
        remove_tree(staging)
        try:
            fetch(staging, name, url, commit, submodules)
        except BaseException:
            remove_tree(staging)
            raise
        staging.rename(folder)
    git = ["git", "-c", "core.autocrlf=false", "-C", str(folder)]
    head = subprocess.run([*git, "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run([*git, "log", "-1", "--format=%cs"], capture_output=True, text=True, check=True).stdout.strip()
    print(f"{name:<14} {head} {date}")


def fetch(folder: Path, name: str, url: str, commit: str, submodules: bool) -> None:
    """Makes `folder` (a new one) the checkout of `commit` of `url`, with the patch of `name` applied if there is one."""
    git = ["git", "-c", "core.autocrlf=false", "-C", str(folder)]
    folder.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", str(folder)], check=True)
    subprocess.run([*git, "fetch", "-q", "--depth", "1", url, commit], check=True)
    subprocess.run([*git, "checkout", "-q", "FETCH_HEAD"], check=True)
    if submodules:
        subprocess.run([*git, "submodule", "update", "-q", "--init", "--recursive", "--depth", "1"], check=True)
    patch = HERE / "patches" / f"{name}.patch"
    if patch.is_file():
        subprocess.run([*git, "apply", str(patch)], check=True)


def remove_tree(folder: Path) -> None:
    """Removes `folder` and what is in it, if there is one; git's read-only object files are made writable first."""

    def writable(func, path, _exc) -> None:
        os.chmod(path, stat.S_IWRITE)
        func(path)

    if folder.exists():
        shutil.rmtree(folder, onerror=writable)
