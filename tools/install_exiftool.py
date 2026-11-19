#!/usr/bin/env python3
"""Installs ExifTool on Linux at the release the Justfile pins (exiftool_source_*): the release's source archive is fetched
from its URL and checked against its SHA-256 before anything is unpacked. The `exiftool` program and its lib folder go to
PREFIX, where the program finds its modules beside it; put PREFIX on PATH. The version the program reports must be the
pinned one. Used by the container (docker/Dockerfile, which passes the pins as arguments) and by CI's Linux job (its pins
step writes them). Standard library only.

    python3 tools/install_exiftool.py --url URL --sha256 HEX --version 13.59 --prefix /opt/exiftool
"""

import argparse
import hashlib
import io
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path


def install(url: str, sha256: str, version: str, prefix: Path) -> None:
    with urllib.request.urlopen(url, timeout=120) as response:  # noqa: S310 (the URL is the Justfile's pin)
        archive = response.read()
    digest = hashlib.sha256(archive).hexdigest()
    if digest != sha256:
        sys.exit(f"the ExifTool archive from {url} has SHA-256 {digest}; the Justfile pins {sha256}")
    with tempfile.TemporaryDirectory() as work:
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            tar.extractall(work, filter="data")
        (source,) = [path for path in Path(work).iterdir() if path.is_dir()]
        prefix.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / "exiftool", prefix / "exiftool")
        (prefix / "exiftool").chmod(0o755)
        shutil.rmtree(prefix / "lib", ignore_errors=True)
        shutil.copytree(source / "lib", prefix / "lib")
    # The program loads its modules when it starts, so this fails when the copy is incomplete.
    reported = subprocess.run([str(prefix / "exiftool"), "-ver"], capture_output=True, text=True, check=True).stdout.strip()
    if reported != version:
        sys.exit(f"{prefix / 'exiftool'} reports ExifTool {reported}; the Justfile pins {version}")
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
