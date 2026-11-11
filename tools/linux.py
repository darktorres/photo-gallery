#!/usr/bin/env python3
"""The Linux side of the repository, in the pg-dev container (docker/): the checks of CI's Linux job, the callgrind
benches (`just perf`), the fuzzer (`just fuzz-smoke`) and the eval's Linux parts (`just linux-eval`). Docker Desktop must
be running.

    linux.py build                 build the images: `pg-dev` (the checks), `pg-tools` (the benches and the fuzzer) and
                                   `pg-eval` (the eval's Linux parts)
    linux.py check [--head]        docker/check.py on the working tree, or on the committed HEAD with --head
    linux.py perf [ARGS...]        tools/perf_check.py: the callgrind benches of HEAD, gated against docs/perf/baseline.json
    linux.py bench [ARGS...]       tools/profiling/bench.py: the benches without the gate (gungraun's options)
    linux.py fuzz [SECONDS]        tools/fuzz_smoke.py: each fuzz target for SECONDS (default 60)
    linux.py eval SCRIPT [ARGS...] an eval script (a path under eval/), on the working tree, with the pg-eval volume
    linux.py ann-worker JOB        eval/ann/cpu_worker.py on one job file (eval/ann/methods.py runs it)
    linux.py gallery OUT           a headless Linux `gallery` (release, no desktop) of the working tree, copied into OUT,
                                   to run elsewhere (WSL runs binaries, and builds nothing)
    linux.py shell                 a shell in the checks image, on the working tree

Each command runs in a new container, removed when the command ends, so its output is in that run's logs and nothing is
left running. The compiled state lives in Docker's named volumes (or, with PG_LINUX_CACHE=<folder>, folders under it),
which outlive the containers: a new container reuses the build of the last one.

The tool versions are read from the Justfile with `just --evaluate`, and the gungraun version from Cargo.lock, so each
version is written once. PG_LINUX_BUILD_CACHE=gha builds with the GitHub Actions cache (CI). CI restores PG_LINUX_CACHE
with actions/cache.
"""

import argparse
import itertools
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
EVAL = "pg-eval"
# The eval image's Python packages, at the versions eval/requirements.txt pins (the Windows eval venv's), so each version is
# written once: numpy, FAISS and USearch for the ANN worker, Pillow for the JPEG encoder comparison.
EVAL_PACKAGES = ("numpy", "faiss-cpu", "usearch", "pillow")
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
# The committed HEAD, extracted for the runs that take it. Every container mounts this folder at /head; `refresh_head`
# refills it, and it is a fixed folder so the mount never changes.
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


def eval_packages():
    """The pins of EVAL_PACKAGES in eval/requirements.txt (`name==version`), space-separated."""
    pins = {}
    for line in (REPO / "eval" / "requirements.txt").read_text(encoding="utf-8").splitlines():
        name, sep, version = line.strip().partition("==")
        if sep:
            pins[name.lower()] = f"{name}=={version}"
    missing = [name for name in EVAL_PACKAGES if name not in pins]
    if missing:
        fail(f"eval/requirements.txt pins no {', '.join(missing)}")
    return " ".join(pins[name] for name in EVAL_PACKAGES)


def build_args():
    args = []
    for arg, variable in PINS.items():
        args += ["--build-arg", f"{arg}={just_value(variable)}"]
    return [
        *args,
        "--build-arg",
        f"GUNGRAUN_VERSION={gungraun_version()}",
        "--build-arg",
        f"EVAL_PACKAGES={eval_packages()}",
    ]


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
    Path(path).chmod(stat.S_IWRITE)
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


def mount_sources(mounts_list):
    """The host folders among docker run's `-v SOURCE:TARGET[:MODE]` arguments: the Windows drive paths (`D:/x`), not the
    named volumes."""
    sources = []
    for flag, value in itertools.pairwise(mounts_list):
        if flag == "-v" and re.match(r"^[A-Za-z]:[/\\]", value):
            end = value.find(":", 2)
            sources.append(value[:end] if end != -1 else value)
    return sources


def unshared(sources, shared):
    """The folders of `sources` under none of the `shared` folders (compared as Windows compares paths: without case,
    either separator)."""

    def norm(path):
        return os.path.normcase(os.path.normpath(path)).rstrip("\\/")

    roots = [norm(folder) for folder in shared]
    return [s for s in sources if not any(norm(s) == r or norm(s).startswith(r + os.sep) for r in roots)]


def docker_desktop_shared():
    """The folders Docker Desktop shares with its containers (`FilesharingDirectories` in its settings), or None where
    there is no such list (another host, or a settings file without it): then nothing is checked."""
    if sys.platform != "win32" or not os.environ.get("APPDATA"):
        return None
    settings = Path(os.environ["APPDATA"]) / "Docker" / "settings-store.json"
    try:
        shared = json.loads(settings.read_text(encoding="utf-8")).get("FilesharingDirectories")
    except (OSError, ValueError):
        return None
    return shared if isinstance(shared, list) else None


def shared_or_fail(mounts_list):
    """Fails at once when a host folder to mount is one Docker Desktop doesn't share: `docker run` would otherwise wait,
    with no message here, for its "share this folder?" approval in Docker Desktop's window."""
    shared = docker_desktop_shared()
    if shared is None:
        return
    missing = unshared(mount_sources(mounts_list), shared)
    if missing:
        fail(
            f"Docker Desktop doesn't share {', '.join(missing)}: add it (Settings > Resources > File sharing), or use a "
            "folder under one it shares"
        )


def run(image, command, mounts_list, flags=(), env=(), workdir="/work", tty=False):
    """Runs `command` in a new container of `image`, removed when it ends. `mounts_list` and `flags` are docker run's; each
    name in `env` is passed on from this process, and each `NAME=value` is set to that value. A host folder Docker
    Desktop doesn't share fails first ([`shared_or_fail`])."""
    shared_or_fail(list(mounts_list))
    interactive = ["-it"] if tty and sys.stdin.isatty() and sys.stdout.isatty() else []
    env_flags = [item for var in env for item in ("-e", var)]
    cmd = ["docker", "run", "--rm", *interactive, "-w", workdir, *env_flags, *flags, *mounts_list, image, *command]
    return subprocess.run(cmd, check=False).returncode


def check(head):
    ensure_image(CHECKS, "base")
    if head:
        refresh_head()
    return run(CHECKS, ["python3", "docker/check.py"], mounts(), workdir="/head" if head else "/work")


def head_run(tag, target, command, flags=(), extra_mounts=(), env=()):
    """Runs a tools command in a new container of `tag`: the launcher's tree at /work (the scripts), the committed HEAD at
    /head (what the tool benches or fuzzes), and PG_HEAD_SHA set to HEAD's short hash."""
    ensure_image(tag, target)
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True, check=True)
    os.environ["PG_HEAD_SHA"] = sha.stdout.strip()
    refresh_head()
    return run(tag, command, [*mounts(), *extra_mounts], flags=flags, env=env)


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
    sys.exit(head_run(TOOLS, "tools", command))


# The eval's folders in the container: the pg-eval volume (or PG_LINUX_CACHE/eval) at /eval, which keeps the sources,
# builds, inputs and clone the scripts make between runs. Two host folders from .env are mounted when set: the photos the
# JPEG comparison starts from (JPEG_SRC_DIR, read-only) and the JPEG encoder comparison's work folder (PG_JPEG_ENC_DIR), which
# gpu_encoders.py on Windows writes into too.
EVAL_ENV = [
    "JPEG_EVAL=/eval/jpeg-eval",
    "JPEG_INPUTS=/eval/jpeg-inputs",
    "JPEG_RESTART_OUT=/eval/jpeg-restart",
    "PG_BENCH_CLONE=/eval/photo-gallery",
]


def eval_mounts_and_env():
    cache = os.environ.get("PG_LINUX_CACHE")
    if cache:
        (Path(cache) / "eval").mkdir(parents=True, exist_ok=True)
        volume = posix(Path(cache) / "eval")
    else:
        volume = "pg-eval"
    mounts_list = ["-v", f"{volume}:/eval"]
    env = list(EVAL_ENV)
    encodes = os.environ.get("PG_JPEG_ENC_DIR", "")
    if encodes:
        Path(encodes).mkdir(parents=True, exist_ok=True)
        mounts_list += ["-v", f"{posix(encodes)}:/jpeg-enc"]
        env.append("PG_JPEG_ENC_DIR=/jpeg-enc")
    else:
        env.append("PG_JPEG_ENC_DIR=/eval/jpeg-enc")
    source = os.environ.get("JPEG_SRC_DIR", "")
    if source:
        if not Path(source).is_dir():
            fail(f"JPEG_SRC_DIR is not a folder: {source}")
        mounts_list += ["-v", f"{posix(source)}:/jpeg-src:ro"]
        env.append("JPEG_SRC_DIR=/jpeg-src")
    return mounts_list, env


def eval_script(script, args):
    """Runs an eval script on the working tree, in the eval image's Python, with the bench flags (the JPEG comparison runs
    callgrind through gungraun)."""
    path = (REPO / script).resolve()
    if not path.is_file() or not path.is_relative_to(REPO / "eval"):
        fail(f"not a script under eval/: {script}")
    ensure_image(EVAL, "eval")
    extra, env = eval_mounts_and_env()
    command = ["/opt/eval/bin/python", path.relative_to(REPO).as_posix(), *args]
    sys.exit(run(EVAL, command, [*mounts(), *extra], flags=BENCH_FLAGS, env=env))


# The job keys of eval/ann/cpu_worker.py that name files (and `out`, a folder).
ANN_PATH_KEYS = ("base", "queries", "train", "centroids", "out")
# FAISS's own OpenMP threads plus a multithreaded BLAS oversubscribe the 12 threads: OPQ training (10k x 512, 4
# iterations) took >53 s that way, 25.9 s with BLAS on one thread, 16.6 s with passive OpenMP waits as well.
ANN_ENV = ["OPENBLAS_NUM_THREADS=1", "OMP_WAIT_POLICY=PASSIVE"]


def ann_worker(job_file):
    """Runs cpu_worker.py on `job_file`, a job that names this machine's paths. Each path's folder is mounted (`out`
    read-write, the inputs read-only) at /ann/<n>, and the worker reads a copy of the job with those paths, written into
    `out` as job.container.json."""
    job = json.loads(Path(job_file).read_text(encoding="utf-8"))
    out = Path(job["out"])
    folders = {out: "/ann/0"}
    mounts_list = ["-v", f"{posix(out)}:/ann/0"]
    container_job = dict(job, out="/ann/0")
    for key in ANN_PATH_KEYS[:-1]:
        if not job.get(key):
            continue
        path = Path(job[key])
        folder = path.parent
        if folder not in folders:
            folders[folder] = f"/ann/{len(folders)}"
            mounts_list += ["-v", f"{posix(folder)}:{folders[folder]}:ro"]
        container_job[key] = f"{folders[folder]}/{path.name}"
    (out / "job.container.json").write_text(json.dumps(container_job), encoding="utf-8")
    ensure_image(EVAL, "eval")
    command = ["/opt/eval/bin/python", "eval/ann/cpu_worker.py", "/ann/0/job.container.json"]
    sys.exit(run(EVAL, command, ["-v", f"{posix(REPO)}:/work:ro", *mounts_list], env=ANN_ENV))


def gallery(out):
    """Builds the command-line `gallery` (release, `--no-default-features`: no Qt) of the working tree in the checks image,
    and copies it into the host folder `out`, which is made if it is missing."""
    ensure_image(CHECKS, "base")
    folder = Path(out).resolve()
    folder.mkdir(parents=True, exist_ok=True)
    script = (
        "cargo build --release --locked -p pg-gallery --no-default-features --bin gallery && cp "
        "/cargo-target/release/gallery /out/gallery"
    )
    code = run(CHECKS, ["bash", "-c", script], [*mounts(), "-v", f"{posix(folder)}:/out"])
    if code == 0:
        print(f"gallery: {folder / 'gallery'}")
    sys.exit(code)


def shell():
    ensure_image(CHECKS, "base")
    sys.exit(run(CHECKS, ["bash"], mounts(), tty=True))


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
    # eval hands the arguments after the script to it, as perf and bench do.
    eval_parser = commands.add_parser("eval", help="an eval script, e.g. eval/jpeg-decoders/linux.py compare")
    eval_parser.add_argument("script", help="the script, a path under eval/")
    ann_parser = commands.add_parser("ann-worker", help="eval/ann/cpu_worker.py on one job file")
    ann_parser.add_argument("job", help="the job file (job.json), with this machine's paths")
    gallery_parser = commands.add_parser("gallery", help="a headless Linux gallery binary, copied into a host folder")
    gallery_parser.add_argument("out", help="the host folder to copy it into")
    commands.add_parser("shell", help="a shell in the checks image")
    options, rest = parser.parse_known_args()
    if rest and options.command not in ("perf", "bench", "eval"):
        parser.error(f"unrecognized arguments: {' '.join(rest)}")

    if options.command == "build":
        build("base", CHECKS)
        build("tools", TOOLS)
        build("eval", EVAL)
    elif options.command == "check":
        sys.exit(check(options.head))
    elif options.command == "perf":
        perf(rest)
    elif options.command == "bench":
        bench(rest)
    elif options.command == "fuzz":
        fuzz(options.seconds)
    elif options.command == "eval":
        eval_script(options.script, rest)
    elif options.command == "ann-worker":
        ann_worker(options.job)
    elif options.command == "gallery":
        gallery(options.out)
    elif options.command == "shell":
        shell()


if __name__ == "__main__":
    main()
