#!/usr/bin/env python3
"""The Linux side of the repository, in the pg-dev container (docker/): the checks of CI's Linux job, the callgrind
benches (`just perf`) and the fuzzer (`just fuzz-smoke`). Docker Desktop must be running.

    linux.py build                 build the images: `pg-dev` (the checks) and `pg-tools` (the benches and the fuzzer)
    linux.py check [--head]        docker/check.py on the working tree, or on the committed HEAD with --head
    linux.py perf [ARGS...]        tools/perf_check.py: the callgrind benches of HEAD, gated against docs/perf/baseline.json
    linux.py bench [ARGS...]       tools/profiling/bench.py: the benches without the gate (gungraun's options)
    linux.py fuzz [SECONDS]        tools/fuzz_smoke.py: each fuzz target for SECONDS (default 60)
    linux.py shell                 a shell in `pg-dev`, on the working tree

The tool versions are read from the Justfile with `just --evaluate`, and the gungraun version from Cargo.lock, so each
version is written once. The volumes are Docker's named volumes, or, with PG_LINUX_CACHE=<folder>, folders under it (CI
restores them with actions/cache). PG_LINUX_BUILD_CACHE=gha builds with the GitHub Actions cache (CI).
"""

import argparse
import contextlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path
from typing import NoReturn

REPO = Path(__file__).resolve().parent.parent
CHECKS = "pg-dev"
TOOLS = "pg-tools"
# The Justfile's variable behind each build argument of docker/Dockerfile.
PINS = {
    "JUST_VERSION": "just_version",
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


@contextlib.contextmanager
def head_export():
    """A folder holding the committed HEAD (`export_head`), removed afterwards. It is made with a plain `mkdir`, under
    target/: since Python 3.12.4, `tempfile.mkdtemp` gives its folder an ACL that admits only this user on Windows, and
    Docker Desktop, whose service mounts it, is then refused ("Access denied")."""
    folder = REPO / "target" / "linux-head" / f"pg-head-{os.getpid()}"
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    try:
        export_head(folder)
        yield folder
    finally:
        shutil.rmtree(folder, ignore_errors=True)


def run(image, command, mounts, extra=()):
    interactive = ["-it"] if sys.stdin.isatty() and sys.stdout.isatty() else []
    cmd = ["docker", "run", "--rm", *interactive, *extra, *mounts, image, *command]
    return subprocess.run(cmd, check=False).returncode


def check(head):
    ensure_image(CHECKS, "base")
    if not head:
        return run(CHECKS, ["python3", "docker/check.py"], ["-v", f"{posix(REPO)}:/work", *cache_mounts()])
    with head_export() as export:
        return run(CHECKS, ["python3", "docker/check.py"], ["-v", f"{posix(export)}:/work", *cache_mounts()])


def with_head(command, extra_mounts, extra_env=(), extra_flags=()):
    """Runs `command` in the tools image: the checkout at /work, a committed HEAD export at /head, the caches."""
    ensure_image(TOOLS, "tools")
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True, check=True)
    os.environ["PG_HEAD_SHA"] = sha.stdout.strip()
    with head_export() as export:
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
        ["python3", "tools/perf_check.py", *args],
        ["-v", f"{posix(source)}:/bench-src:ro"],
        extra_env=["PG_BENCH_SKIP_CATALOG", "PG_HEAD_SHA"],
        extra_flags=BENCH_FLAGS,
    )
    sys.exit(code)


def bench(args):
    code = with_head(["python3", "tools/profiling/bench.py", *args], [], extra_env=["PG_HEAD_SHA"], extra_flags=BENCH_FLAGS)
    sys.exit(code)


def fuzz(seconds):
    command = ["python3", "tools/fuzz_smoke.py", *([seconds] if seconds else [])]
    sys.exit(with_head(command, []))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("build", help="build the images")
    check_parser = commands.add_parser("check", help="the Linux job's checks")
    check_parser.add_argument("--head", action="store_true", help="check the committed HEAD, not the working tree")
    # perf and bench hand everything after their name to a script in the container. Those options are the script's, so
    # they are not declared here: an `argparse.REMAINDER` positional rejects a first token that starts with `--` (the
    # documented `just perf --update`), so the two commands take what parse_known_args leaves.
    commands.add_parser("perf", help="the callgrind benches, gated (tools/perf_check.py's options, e.g. --update)")
    commands.add_parser("bench", help="the callgrind benches, ungated (gungraun's options, e.g. --save-baseline=x)")
    fuzz_parser = commands.add_parser("fuzz", help="each fuzz target for a short time")
    fuzz_parser.add_argument("seconds", nargs="?", default="", help="seconds per target (default 60)")
    commands.add_parser("shell", help="a shell in the checks image")
    options, rest = parser.parse_known_args()
    if rest and options.command not in ("perf", "bench"):
        parser.error(f"unrecognized arguments: {' '.join(rest)}")

    if options.command == "build":
        build("base", CHECKS)
        build("tools", TOOLS)
    elif options.command == "check":
        sys.exit(check(options.head))
    elif options.command == "perf":
        perf(rest)
    elif options.command == "bench":
        bench(rest)
    elif options.command == "fuzz":
        fuzz(options.seconds)
    elif options.command == "shell":
        ensure_image(CHECKS, "base")
        sys.exit(run(CHECKS, ["bash"], ["-v", f"{posix(REPO)}:/work", *cache_mounts()]))


if __name__ == "__main__":
    main()
