#!/usr/bin/env python3
"""The Linux side of the repository, in the pg-dev container (docker/): the checks of CI's Linux job, the callgrind
benches (`just perf`) and the fuzzer (`just fuzz-smoke`). Docker Desktop must be running.

    linux.py build                 build the images: `pg-dev` (the checks) and `pg-tools` (the benches and the fuzzer)
    linux.py check [--head]        docker/check.py on the working tree, or on the committed HEAD with --head
    linux.py perf [ARGS...]        tools/perf_check.py: the callgrind benches of HEAD, gated against docs/perf/baseline.json
    linux.py bench [ARGS...]       tools/profiling/bench.py: the benches without the gate (gungraun's options)
    linux.py fuzz [SECONDS]        tools/fuzz_smoke.py: each fuzz target for SECONDS (default 60)
    linux.py shell                 a shell in the checks container, on the working tree

The containers are persistent: each is created once, kept running, and the commands run in it with `docker exec`. There
are three, one per image and security profile: `pg-checks-box` (pg-dev, the checks and the shell), `pg-tools-box`
(pg-tools, the fuzzer) and `pg-bench-box` (pg-tools with seccomp unconfined, which gungraun's valgrind needs). A container
is recreated only when its image, its mounts or its flags change; `docker rm -f <name>` removes one by hand.

The tool versions are read from the Justfile with `just --evaluate`, and the gungraun version from Cargo.lock, so each
version is written once. The volumes are Docker's named volumes, or, with PG_LINUX_CACHE=<folder>, folders under it (CI
restores them with actions/cache). PG_LINUX_BUILD_CACHE=gha builds with the GitHub Actions cache (CI).
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

REPO = Path(__file__).resolve().parent.parent
CHECKS = "pg-dev"
TOOLS = "pg-tools"
# The persistent containers (see the module docstring). The label `pg.spec` on each holds the hash of its image, mounts
# and flags, so a container made for something else is recreated.
CHECKS_BOX = "pg-checks-box"
TOOLS_BOX = "pg-tools-box"
BENCH_BOX = "pg-bench-box"
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
# The committed HEAD, extracted for the runs that take it. Every container mounts this folder at /head, so it is one fixed
# folder: `refresh_head` refills it, and no container is recreated for a new HEAD.
HEAD_FOLDER = REPO / "target" / "linux-head"


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


def docker(*args):
    """`docker` with its output captured: (exit code, stdout, stripped)."""
    result = subprocess.run(["docker", *args], capture_output=True, text=True, check=False)
    return result.returncode, result.stdout.strip()


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


def mounts():
    """What every container has: the checkout at /work, the committed HEAD folder at /head, and the caches."""
    HEAD_FOLDER.mkdir(parents=True, exist_ok=True)
    return ["-v", f"{posix(REPO)}:/work", "-v", f"{posix(HEAD_FOLDER)}:/head", *cache_mounts()]


def export_head(destination):
    """The committed HEAD, as a shallow clone of this repository in `destination`: what is committed, and its own .git, so
    git works there as it does in the checkout (the scripts ask git for the tracked files). The clone takes the commit's
    objects only (--depth 1), not the working tree's uncommitted work."""
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, capture_output=True, text=True, check=False)
    if head.returncode != 0:
        fail(f"git rev-parse HEAD failed: {head.stderr.strip()}")
    clone = subprocess.run(
        ["git", "clone", "--quiet", "--no-checkout", "--depth", "1", REPO.as_uri(), str(destination)],
        capture_output=True,
        text=True,
        check=False,
    )
    if clone.returncode != 0:
        fail(f"git clone of HEAD failed: {clone.stderr.strip()}")
    checkout = subprocess.run(
        ["git", "-C", str(destination), "checkout", "--quiet", "--detach", head.stdout.strip()],
        capture_output=True,
        text=True,
        check=False,
    )
    if checkout.returncode != 0:
        fail(f"git checkout of HEAD failed: {checkout.stderr.strip()}")


def _clear_read_only(function, path, _error):
    """rmtree's error handler: git's object files are read-only on Windows, so they are made writable and removed."""
    os.chmod(path, stat.S_IWRITE)
    function(path)


def refresh_head():
    """Refills HEAD_FOLDER with the committed HEAD. The folder itself stays, since the containers have it mounted; its
    contents are replaced. It is made with a plain `mkdir`, not `tempfile`: since Python 3.12.4, `tempfile.mkdtemp` gives
    its folder an ACL that admits only this user on Windows, and Docker Desktop, whose service mounts it, is then refused
    ("Access denied")."""
    HEAD_FOLDER.mkdir(parents=True, exist_ok=True)
    for entry in HEAD_FOLDER.iterdir():
        if entry.is_dir() and not entry.is_symlink():
            shutil.rmtree(entry, onexc=_clear_read_only)
        else:
            entry.unlink(missing_ok=True)
    export_head(HEAD_FOLDER)


def ensure_box(name, tag, target, flags, box_mounts):
    """The persistent container `name` is running, on the image `tag` (built when missing). It is created once, with
    `sleep infinity` as its command, and kept. It is recreated when its image, mounts or flags differ from the label
    `pg.spec` it was made with."""
    ensure_image(tag, target)
    image_id = docker("image", "inspect", "-f", "{{.Id}}", tag)[1]
    spec = hashlib.sha256(json.dumps([image_id, flags, box_mounts]).encode()).hexdigest()
    code, current = docker("container", "inspect", "-f", '{{index .Config.Labels "pg.spec"}}', name)
    if code == 0 and current == spec:
        if docker("container", "inspect", "-f", "{{.State.Running}}", name)[1] != "true":
            started = subprocess.run(["docker", "start", name], capture_output=True, text=True, check=False)
            if started.returncode != 0:
                fail(f"could not start {name}: {started.stderr.strip()}")
        return
    if code == 0:
        docker("rm", "-f", name)
    created = subprocess.run(
        ["docker", "run", "-d", "--name", name, "--label", f"pg.spec={spec}", *flags, *box_mounts, tag, "sleep", "infinity"],
        capture_output=True,
        text=True,
        check=False,
    )
    if created.returncode != 0:
        fail(f"could not create {name}: {created.stderr.strip()}")


def exec_in(name, command, workdir, env=(), tty=False):
    """Runs `command` in the container `name`, in `workdir`; each name in `env` is passed on from this process."""
    interactive = ["-it"] if tty and sys.stdin.isatty() and sys.stdout.isatty() else []
    env_flags = [item for var in env for item in ("-e", var)]
    return subprocess.run(["docker", "exec", *interactive, "-w", workdir, *env_flags, name, *command], check=False).returncode


def head_run(box, tag, target, command, flags=(), extra_mounts=(), env=()):
    """Runs a tools command in the container `box`: the launcher's tree at /work (the scripts), the committed HEAD at /head
    (what the tool benches or fuzzes), and PG_HEAD_SHA set to HEAD's short hash."""
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True, check=True)
    os.environ["PG_HEAD_SHA"] = sha.stdout.strip()
    refresh_head()
    ensure_box(box, tag, target, list(flags), [*mounts(), *extra_mounts])
    return exec_in(box, command, "/work", env=env)


def check(head):
    ensure_box(CHECKS_BOX, CHECKS, "base", [], mounts())
    if head:
        refresh_head()
    return exec_in(CHECKS_BOX, ["python3", "docker/check.py"], "/head" if head else "/work")


# Gungraun runs valgrind under `setarch -R` (ASLR off, as the baseline was measured), which needs the personality
# syscall with ADDR_NO_RANDOMIZE; Docker's default seccomp profile refuses it. Only the bench runs get this.
BENCH_FLAGS = ["--security-opt", "seccomp=unconfined"]


def bench_source_mounts():
    """The bench data folder (PG_BENCH_DATA_SOURCE in .env), read-only at /bench-src, when it is set and exists."""
    source = os.environ.get("PG_BENCH_DATA_SOURCE", "")
    if source and Path(source).is_dir():
        return ["-v", f"{posix(source)}:/bench-src:ro"]
    return []


def perf(args):
    if not bench_source_mounts():
        fail("PG_BENCH_DATA_SOURCE must name the bench data folder (.env, docs/PROFILING.md)")
    code = head_run(
        BENCH_BOX,
        TOOLS,
        "tools",
        ["python3", "tools/perf_check.py", *args],
        flags=BENCH_FLAGS,
        extra_mounts=bench_source_mounts(),
        env=["PG_BENCH_SKIP_CATALOG", "PG_HEAD_SHA"],
    )
    sys.exit(code)


def bench(args):
    code = head_run(
        BENCH_BOX,
        TOOLS,
        "tools",
        ["python3", "tools/profiling/bench.py", *args],
        flags=BENCH_FLAGS,
        extra_mounts=bench_source_mounts(),
        env=["PG_HEAD_SHA"],
    )
    sys.exit(code)


def fuzz(seconds):
    command = ["python3", "tools/fuzz_smoke.py", *([seconds] if seconds else [])]
    sys.exit(head_run(TOOLS_BOX, TOOLS, "tools", command))


def shell():
    ensure_box(CHECKS_BOX, CHECKS, "base", [], mounts())
    sys.exit(exec_in(CHECKS_BOX, ["bash"], "/work", tty=True))


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
    commands.add_parser("shell", help="a shell in the checks container")
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
        shell()


if __name__ == "__main__":
    main()
