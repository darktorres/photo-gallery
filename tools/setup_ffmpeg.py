#!/usr/bin/env python3
"""Fetches the pinned FFmpeg build for `just setup`: downloads the archive once (into .deps/downloads), checks its SHA-256
against the pin, and unpacks it to --destination. The folder is skipped only when it holds a build at this pin (the headers
and the `.pin` record the unpack writes); a build there at another pin, or one without the record, is replaced. The Justfile
holds the URL and the hash; CI runs the same recipe.

A download that does not match the hash is deleted and the script fails, so a bad archive is never unpacked or cached.
Windows only: Linux gets FFmpeg's development packages from apt, which crates/video/build.rs finds through pkg-config.
"""

import argparse
import shutil
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile
from pathlib import Path, PurePosixPath

from pgtools import REPO, sha256

# A shared FFmpeg build has these headers; the check that the archive is one.
HEADER = Path("include") / "libavcodec" / "avcodec.h"
# The pin the build was fetched at, written beside it. The header alone says only that a build is there, not which.
PIN_FILE = ".pin"
CHUNK = 1 << 20


def download(url: str, target: Path) -> None:
    """Fetches `url` to `target`, through a `.partial` name so an interrupted download never looks finished."""
    partial = target.with_name(f"{target.name}.{uuid.uuid4().hex}.partial")
    print(f"Downloading {url}", flush=True)
    if not url.startswith("https://"):
        sys.exit(f"the FFmpeg archive's URL is not https: {url}")
    try:
        # An https URL, checked above: the pinned one, from the Justfile.
        with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as file:  # noqa: S310 (an https URL, checked above)
            shutil.copyfileobj(response, file, CHUNK)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            sys.exit(
                f"the FFmpeg archive is not at {url} (404). BtbN removes its autobuilds after 14 days, except the last "
                "build of each month, which it keeps for two years: repin the Justfile to a month-end build."
            )
        raise
    partial.replace(target)


def installed_pin(dest: Path) -> str | None:
    """The SHA-256 the build in `dest` was fetched at; None when no pin is recorded there."""
    record = dest / PIN_FILE
    return record.read_text(encoding="utf-8").strip() if record.is_file() else None


def is_installed(dest: Path, pin: str) -> bool:
    """Whether `dest` holds a usable build fetched at `pin`."""
    return (dest / HEADER).is_file() and installed_pin(dest) == pin


def entry_parts(name: str) -> tuple[str, ...]:
    """The path parts of an archive entry, with Windows separators read as `/`."""
    return PurePosixPath(name.replace("\\", "/")).parts


def unsafe_name(name: str) -> bool:
    """Whether an archive entry would leave the folder it is unpacked into: an absolute path, a drive, or a `..` part."""
    parts = entry_parts(name)
    return name.replace("\\", "/").startswith("/") or ".." in parts or (bool(parts) and ":" in parts[0])


def top_folder(names: list[str]) -> str:
    """The one folder every entry of the archive is in. Raises ValueError when there is not exactly one."""
    tops = {entry_parts(name)[0] for name in names if entry_parts(name)}
    if len(tops) != 1:
        raise ValueError(f"the FFmpeg archive has {len(tops)} top folders, expected one")
    return tops.pop()


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--url", required=True, help="the archive's URL (the pin is in the Justfile)")
    parser.add_argument("--sha256", required=True, help="the archive's SHA-256, as the Justfile pins it")
    parser.add_argument("--destination", required=True, help="the folder to unpack into, relative to the repository root")
    args = parser.parse_args()

    dest = (REPO / args.destination).resolve()
    # A folder that is replaced is removed whole: a non-empty folder is replaced only when it is an FFmpeg build's.
    if dest.exists() and not (dest / HEADER).is_file() and not (dest / PIN_FILE).is_file():
        if not dest.is_dir() or any(dest.iterdir()):
            sys.exit(
                f"refusing to replace {dest}: it is not an FFmpeg build (no {HEADER} and no {PIN_FILE}); name another folder"
            )
    wanted = args.sha256.lower()
    if is_installed(dest, wanted):
        print(f"FFmpeg is already in {dest} at this pin: nothing to do.")
        return
    if (dest / HEADER).is_file():
        held = installed_pin(dest)
        was = f"was fetched at {held}" if held else "has no pin recorded"
        print(f"FFmpeg in {dest} {was}: replacing it with the pin {wanted}.")

    downloads = REPO / ".deps" / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    zip_path = downloads / PurePosixPath(urllib.parse.urlsplit(args.url).path).name
    # A download left from an earlier run is used if it is intact; otherwise it is fetched again.
    if not (zip_path.is_file() and sha256(zip_path).lower() == wanted):
        zip_path.unlink(missing_ok=True)
        download(args.url, zip_path)
        actual = sha256(zip_path).lower()
        if actual != wanted:
            zip_path.unlink()
            sys.exit(
                f"the FFmpeg archive has SHA-256 {actual}, but the pin is {args.sha256}: the download was deleted "
                "(the Justfile holds the pin)"
            )

    extract = REPO / ".deps" / "extract"
    if extract.exists():
        shutil.rmtree(extract)
    with zipfile.ZipFile(zip_path) as archive:
        names = archive.namelist()
        unsafe = [name for name in names if unsafe_name(name)]
        if unsafe:
            sys.exit(f"the FFmpeg archive has an entry that leaves its folder: {unsafe[0]}")
        try:
            top = top_folder(names)
        except ValueError as error:
            sys.exit(str(error))
        archive.extractall(extract)
    # A partial build, or one at another pin, is replaced: the check above found no build here at this pin.
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(extract / top), str(dest))
    shutil.rmtree(extract)

    if not (dest / HEADER).is_file():
        sys.exit("the archive has no include/libavcodec/avcodec.h: it is not a shared FFmpeg build")
    (dest / PIN_FILE).write_text(wanted + "\n", encoding="utf-8")
    print(f"FFmpeg is in {dest}, at the pin {wanted}.")


if __name__ == "__main__":
    main()
