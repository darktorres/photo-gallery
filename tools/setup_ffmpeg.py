#!/usr/bin/env python3
"""Fetches the pinned FFmpeg build for `just setup`: downloads the archive once (into .deps/downloads), checks its SHA-256
against the pin, and unpacks it to --destination. The folder is skipped when its headers are already there. The Justfile
holds the URL and the hash; CI runs the same recipe.

A download that does not match the hash is deleted and the script fails, so a bad archive is never unpacked or cached.
Windows only: Linux gets FFmpeg's development packages from apt, which crates/video/build.rs finds through pkg-config.
"""

import argparse
import hashlib
import shutil
import sys
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

from pgtools import REPO

# A shared FFmpeg build has these headers; the check that the archive is one.
HEADER = Path("include") / "libavcodec" / "avcodec.h"
CHUNK = 1 << 20


def sha256_of(path: Path) -> str:
    """The SHA-256 of a file, as lowercase hex."""
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url: str, target: Path) -> None:
    """Fetches `url` to `target`, through a `.partial` name so an interrupted download never looks finished."""
    partial = target.with_name(target.name + ".partial")
    print(f"Downloading {url}", flush=True)
    with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as file:
        shutil.copyfileobj(response, file, CHUNK)
    partial.replace(target)


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
    if (dest / HEADER).is_file():
        print(f"FFmpeg is already in {dest}: nothing to do (delete the folder to fetch it again).")
        return

    downloads = REPO / ".deps" / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    zip_path = downloads / PurePosixPath(urllib.parse.urlsplit(args.url).path).name
    # A download left from an earlier run is used if it is intact; otherwise it is fetched again.
    if not (zip_path.is_file() and sha256_of(zip_path) == args.sha256.lower()):
        zip_path.unlink(missing_ok=True)
        download(args.url, zip_path)
        actual = sha256_of(zip_path)
        if actual != args.sha256.lower():
            zip_path.unlink()
            sys.exit(
                f"the FFmpeg archive has SHA-256 {actual}, but the pin is {args.sha256}: the download was deleted "
                "(the Justfile holds the pin)"
            )

    extract = REPO / ".deps" / "extract"
    shutil.rmtree(extract, ignore_errors=True)
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
    # Only a partial or older copy is replaced: the header check above found no usable build here.
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(extract / top), str(dest))
    shutil.rmtree(extract, ignore_errors=True)

    if not (dest / HEADER).is_file():
        sys.exit("the archive has no include/libavcodec/avcodec.h: it is not a shared FFmpeg build")
    print(f"FFmpeg is in {dest}.")


if __name__ == "__main__":
    main()
