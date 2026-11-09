#!/usr/bin/env python3
"""The system setup for CI's Linux job (.github/workflows/ci.yml): the apt packages (docker/apt-packages.txt, the list the
container installs too), mold, the Rust components. FFmpeg's headers come from its development packages, which pkg-config
finds. Idempotent.
"""

import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CARGO_BIN = Path.home() / ".cargo" / "bin"


def apt(*arguments: str) -> list[str]:
    """An apt-get command, as root, non-interactive. `env` sets the variable after sudo has reset the environment."""
    prefix = [] if os.geteuid() == 0 else ["sudo"]
    return [*prefix, "env", "DEBIAN_FRONTEND=noninteractive", "apt-get", *arguments]


def package_list() -> list[str]:
    """The apt packages in docker/apt-packages.txt: its lines that are neither comments nor blank."""
    lines = (REPO / "docker" / "apt-packages.txt").read_text(encoding="utf-8").splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]


def install_packages() -> None:
    # A distro with foreign architectures enabled gets 404s for their indexes; amd64's still update.
    print("== apt: package index", flush=True)
    update = subprocess.run(apt("update"), check=False)
    if update.returncode != 0:
        print("apt-get update: some indexes failed (ignored)", flush=True)
    # libssl-dev: ureq's native-tls is OpenSSL on Linux. libclang-dev: bindgen (pg-video's build script). ffmpeg: the command
    # the video tests run (pg_library's video tests look for it on PATH).
    packages = package_list()
    # qt-build-utils links with lld, gold or mold when one is on PATH (in that order). Without one it warns on every build
    # script run that GNU ld.bfd will likely fail (it does not fail). mold needs GCC 12 or newer, and the runner has GCC 15.
    packages.append("mold")
    print(f"== apt: {len(packages)} packages", flush=True)
    subprocess.run(apt("install", "-y", *packages), check=True)


def install_rust() -> None:
    print("== rustup", flush=True)
    rustup = CARGO_BIN / "rustup"
    if not rustup.is_file():
        # The installer is run by `sh` from its own text (fetched here), with the same arguments as the usual one-liner.
        with urllib.request.urlopen("https://sh.rustup.rs", timeout=60) as response:
            installer = response.read()
        subprocess.run(["sh", "-s", "--", "-y", "--profile", "minimal"], input=installer, check=True)
    # The minimal profile has neither component.
    subprocess.run([str(rustup), "component", "add", "clippy", "rustfmt"], check=True)


def report() -> None:
    rustc = subprocess.run([str(CARGO_BIN / "rustc"), "--version"], capture_output=True, text=True, check=True)
    version = " ".join(rustc.stdout.split()[:2])
    modversion = subprocess.run(["pkg-config", "--modversion", "libavcodec"], capture_output=True, text=True, check=True)
    print(f"{version}, FFmpeg libavcodec {modversion.stdout.strip()}")


def main() -> None:
    if sys.platform != "linux":
        sys.exit("ci_setup.py: CI's Linux job only (apt and the Linux Rust setup)")
    if not shutil.which("apt-get"):
        sys.exit("ci_setup.py: apt-get is not on PATH: this script sets up an apt-based system")
    install_packages()
    install_rust()
    report()


if __name__ == "__main__":
    main()
