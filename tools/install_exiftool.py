#!/usr/bin/env python3
"""Installs ExifTool at the release the Justfile pins (exiftool_version): the release's archive is fetched from its URL and
checked against its SHA-256 before anything is unpacked. Two archives, told apart by their first bytes:

- the source archive (a .tar.gz; exiftool_source_*), on Linux: the `exiftool` program and its lib folder go to PREFIX,
  where the program finds its modules beside it;
- the Windows package (a .zip; exiftool_windows_*), on Windows: its `exiftool(-k).exe` goes to PREFIX as `exiftool.exe`,
  and its `exiftool_files` folder (the bundled perl and lib) beside it, as the package's README says.

Put PREFIX on PATH. The version the program reports must be the pinned one. Used by the container (docker/Dockerfile,
which passes the pins as arguments) and by CI's two jobs (their pins step writes them). Standard library only.

    python3 tools/install_exiftool.py --url URL --sha256 HEX --version 13.64 --prefix /opt/exiftool
"""

import argparse
import hashlib
import io
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

GZIP_MAGIC = b"\x1f\x8b"
ZIP_MAGIC = b"PK\x03\x04"
# The Windows package's program: the "-k" in its name makes it wait for a key at the end, so it is renamed on install.
WINDOWS_PROGRAM = "exiftool(-k).exe"


def unpack(archive: bytes, url: str, work: Path) -> Path:
    """The archive unpacked into `work`; returns the program it installs (in the release's one top folder)."""
    if archive.startswith(GZIP_MAGIC):
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            tar.extractall(work, filter="data")
        name = "exiftool"
    elif archive.startswith(ZIP_MAGIC):
        with zipfile.ZipFile(io.BytesIO(archive)) as package:
            # zipfile drops absolute paths and `..` from member names, so nothing lands outside `work`.
            package.extractall(work)  # noqa: S202 (the archive is the pinned one, its SHA-256 checked above)
        name = WINDOWS_PROGRAM
    else:
        sys.exit(f"the ExifTool archive from {url} is neither a .tar.gz nor a .zip")
    (source,) = [path for path in work.iterdir() if path.is_dir()]
    return source / name


def install(url: str, sha256: str, version: str, prefix: Path) -> None:
    with urllib.request.urlopen(url, timeout=120) as response:  # noqa: S310 (the URL is the Justfile's pin)
        archive = response.read()
    digest = hashlib.sha256(archive).hexdigest()
    if digest != sha256:
        sys.exit(f"the ExifTool archive from {url} has SHA-256 {digest}; the Justfile pins {sha256}")
    with tempfile.TemporaryDirectory() as work:
        source = unpack(archive, url, Path(work))
        if source.name == WINDOWS_PROGRAM:
            program_name, modules = "exiftool.exe", "exiftool_files"
        else:
            program_name, modules = "exiftool", "lib"
        # The release is staged beside the prefix and checked there. Only a program that reports the pinned version
        # replaces the installed one, so a failed check leaves the installed program as it was.
        prefix.parent.mkdir(parents=True, exist_ok=True)
        stage = Path(tempfile.mkdtemp(prefix=".exiftool-stage-", dir=prefix.parent))
        try:
            staged = stage / program_name
            shutil.copy2(source, staged)
            staged.chmod(0o755)
            shutil.copytree(source.parent / modules, stage / modules)
            # The program loads its modules when it starts, so this fails when the copy is incomplete.
            reported = subprocess.run([str(staged), "-ver"], capture_output=True, text=True, check=True).stdout.strip()
            if reported != version:
                sys.exit(f"{staged} reports ExifTool {reported}; the Justfile pins {version}")
            prefix.mkdir(parents=True, exist_ok=True)
            if (prefix / modules).exists():
                shutil.rmtree(prefix / modules)
            shutil.move(str(stage / modules), str(prefix / modules))
            os.replace(staged, prefix / program_name)
        finally:
            shutil.rmtree(stage, ignore_errors=True)
    print(f"ExifTool {reported} installed in {prefix}")


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--url", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--prefix", type=Path, default=Path("/opt/exiftool"))
    args = parser.parse_args()
    install(args.url, args.sha256, args.version, args.prefix)


if __name__ == "__main__":
    main()
