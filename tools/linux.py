#!/usr/bin/env python3
"""The Linux side of the repository, in the pg-dev container (docker/): the checks of CI's Linux job, the callgrind
benches (`just perf`) and the fuzzer (`just fuzz-smoke`). Docker Desktop must be running.

    linux.py build                 build the images: `pg-dev` (the checks) and `pg-tools` (the benches and the fuzzer)
    linux.py check [--head]        docker/check.sh on the working tree, or on the committed HEAD with --head
    linux.py perf [ARGS...]        tools/perf-check.sh: the callgrind benches of HEAD, gated against docs/perf/baseline.json
    linux.py bench [ARGS...]       tools/profiling/bench.sh: the benches without the gate (gungraun's options)
    linux.py fuzz [SECONDS]        tools/fuzz-smoke.sh: each fuzz target for SECONDS (default 60)
    linux.py shell                 a shell in `pg-dev`, on the working tree

The tool versions are read from the Justfile with `just --evaluate`, and the gungraun version from Cargo.lock, so each
version is written once. The volumes are Docker's named volumes, or, with PG_LINUX_CACHE=<folder>, folders under it (CI
restores them with actions/cache). PG_LINUX_BUILD_CACHE=gha builds with the GitHub Actions cache (CI).
"""

import argparse
import os
import re
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import NoReturn

REPO = Path(__file__).resolve().parent.parent
CHECKS = "pg-dev"
TOOLS = "pg-tools"
# The Justfile's variable behind each build argument of docker/Dockerfile.
PINS = {
    "JUST_VERSION": "just_version",
    "RUFF_VERSION": "ruff_version",
    "MARKDOWNLINT_VERSION": "markdownlint_version",
    "CARGO_DENY_VERSION": "cargo_deny_version",
    "CARGO_MACHETE_VERSION": "cargo_machete_version",
    "CARGO_MODULES_VERSION": "cargo_modules_version",
    "CARGO_FUZZ_VERSION": "cargo_fuzz_version",
    "AQTINSTALL_COMMIT": "aqtinstall_commit",
    "QT_VERSION": "qt_linux_version",
}


def fail(message) -> NoReturn:
    sys.exit(f"linux.py: {message}")


def just_value(name):
    result = subprocess.run(["just", "--evaluate", name], cwd=REPO, capture_output=True, text=True, check=False)
    if result.returncode != 0 or not result.stdout.strip():
        fail(f"`just --evaluate {name}` gave no value: {result.stderr.strip()}")
    return result.stdout.strip()


def gungraun_version():
    lock = (REPO / "Cargo.lock").read_text(encoding="utf-8")
    match = re.search(r'name = "gungraun"\nversion = "([^"]+)"', lock)
    if not match:
        fail("Cargo.lock has no gungraun version")
    return match.group(1)


def build_args():
    args = []
    for arg, variable in PINS.items():
        args += ["--build-arg", f"{arg}={just_value(variable)}"]
    return [*args, "--build-arg", f"GUNGRAUN_VERSION={gungraun_version()}"]


def posix(path):
    # Docker takes Windows paths with forward slashes: `D:/GitHub/photo-gallery:/work`.
    return Path(path).as_posix()


def docker_ok(*args):
    return subprocess.run(["docker", *args], capture_output=True, check=False).returncode == 0


def ensure_daemon():
    if not docker_ok("version"):
        fail("Docker is not running: start Docker Desktop, then try again")


def build(target, tag):
    ensure_daemon()
    command = ["docker", "build"]
    if os.environ.get("PG_LINUX_BUILD_CACHE") == "gha":
        command = [
            "docker",
            "buildx",
            "build",
            "--load",
            "--cache-from",
            f"type=gha,scope={tag}",
            "--cache-to",
            f"type=gha,scope={tag},mode=max",
        ]
    command += ["-f", "docker/Dockerfile", "--target", target, "-t", tag, *build_args(), "."]
    result = subprocess.run(command, cwd=REPO, check=False)
    if result.returncode != 0:
        fail(f"the build of {tag} failed")


def ensure_image(tag, target):
    ensure_daemon()
    if not docker_ok("image", "inspect", tag):
        build(target, tag)


def cache_mounts():
    """The Cargo target, registry and git checkouts, and the bench folder: volumes, or folders under PG_LINUX_CACHE."""
    cache = os.environ.get("PG_LINUX_CACHE")
    if cache:
        root = Path(cache)
        names = {"target": "cargo-target", "registry": "registry", "git": "git", "bench": "bench"}
        for name in names.values():
            (root / name).mkdir(parents=True, exist_ok=True)
        sources = {name: posix(root / folder) for name, folder in names.items()}
    else:
        sources = {"target": "pg-cargo-target", "registry": "pg-cargo-registry", "git": "pg-cargo-git", "bench": "pg-bench"}
    return [
        "-v",
        f"{sources['target']}:/cargo-target",
        "-v",
        f"{sources['registry']}:/home/pg/.cargo/registry",
        "-v",
        f"{sources['git']}:/home/pg/.cargo/git",
        "-v",
        f"{sources['bench']}:/bench",
    ]


def export_head(destination):
    """The committed HEAD, extracted into `destination` (git archive: what is committed, nothing else)."""
    archive = subprocess.Popen(["git", "archive", "--format=tar", "HEAD"], cwd=REPO, stdout=subprocess.PIPE)
    with tarfile.open(fileobj=archive.stdout, mode="r|") as tar:
        tar.extractall(destination, filter="data")
    if archive.wait() != 0:
        fail("git archive HEAD failed")


def run(image, command, mounts, extra=()):
    interactive = ["-it"] if sys.stdin.isatty() and sys.stdout.isatty() else []
    cmd = ["docker", "run", "--rm", *interactive, *extra, *mounts, image, *command]
    return subprocess.run(cmd, check=False).returncode


def check(head):
    ensure_image(CHECKS, "base")
    if not head:
        return run(CHECKS, ["bash", "docker/check.sh"], ["-v", f"{posix(REPO)}:/work", *cache_mounts()])
    with tempfile.TemporaryDirectory(prefix="pg-head-") as export:
        export_head(export)
        return run(CHECKS, ["bash", "docker/check.sh"], ["-v", f"{posix(export)}:/work", *cache_mounts()])


def with_head(command, extra_mounts, extra_env=(), extra_flags=()):
    """Runs `command` in the tools image: the checkout at /work, a committed HEAD export at /head, the caches."""
    ensure_image(TOOLS, "tools")
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True, check=True)
    os.environ["PG_HEAD_SHA"] = sha.stdout.strip()
    with tempfile.TemporaryDirectory(prefix="pg-head-") as export:
        export_head(export)
        mounts = ["-v", f"{posix(REPO)}:/work", "-v", f"{posix(export)}:/head", *extra_mounts, *cache_mounts()]
        env = [item for name in extra_env for item in ("-e", name)]
        return run(TOOLS, command, mounts, [*extra_flags, *env])


# Gungraun runs valgrind under `setarch -R` (ASLR off, as the baseline was measured), which needs the personality
# syscall with ADDR_NO_RANDOMIZE; Docker's default seccomp profile refuses it. Only the bench runs get this.
BENCH_FLAGS = ["--security-opt", "seccomp=unconfined"]


def perf(args):
    source = os.environ.get("PG_BENCH_DATA_SOURCE", "")
    if not source or not Path(source).is_dir():
        fail("PG_BENCH_DATA_SOURCE must name the bench data folder (.env, docs/PROFILING.md)")
    code = with_head(
        ["bash", "tools/perf-check.sh", *args],
        ["-v", f"{posix(source)}:/bench-src:ro"],
        extra_env=["PG_BENCH_SKIP_CATALOG", "PG_HEAD_SHA"],
        extra_flags=BENCH_FLAGS,
    )
    sys.exit(code)


def bench(args):
    code = with_head(["bash", "tools/profiling/bench.sh", *args], [], extra_env=["PG_HEAD_SHA"], extra_flags=BENCH_FLAGS)
    sys.exit(code)


def fuzz(seconds):
    command = ["bash", "tools/fuzz-smoke.sh", *([seconds] if seconds else [])]
    sys.exit(with_head(command, []))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("build", help="build the images")
    check_parser = commands.add_parser("check", help="the Linux job's checks")
    check_parser.add_argument("--head", action="store_true", help="check the committed HEAD, not the working tree")
    perf_parser = commands.add_parser("perf", help="the callgrind benches, gated")
    perf_parser.add_argument("args", nargs=argparse.REMAINDER, help="tools/perf-check.sh's options, e.g. --update")
    bench_parser = commands.add_parser("bench", help="the callgrind benches, without the gate (gungraun's options)")
    bench_parser.add_argument("args", nargs=argparse.REMAINDER, help="gungraun's options, e.g. --save-baseline=before")
    fuzz_parser = commands.add_parser("fuzz", help="each fuzz target for a short time")
    fuzz_parser.add_argument("seconds", nargs="?", default="", help="seconds per target (default 60)")
    commands.add_parser("shell", help="a shell in the checks image")
    options = parser.parse_args()

    if options.command == "build":
        build("base", CHECKS)
        build("tools", TOOLS)
    elif options.command == "check":
        sys.exit(check(options.head))
    elif options.command == "perf":
        perf(options.args)
    elif options.command == "bench":
        bench(options.args)
    elif options.command == "fuzz":
        fuzz(options.seconds)
    elif options.command == "shell":
        ensure_image(CHECKS, "base")
        sys.exit(run(CHECKS, ["bash"], ["-v", f"{posix(REPO)}:/work", *cache_mounts()]))


if __name__ == "__main__":
    main()
