# The Linux environment in a container

The container is CI's Linux environment (`.github/workflows/ci.yml`): Ubuntu 26.04, the toolchain from
`rust-toolchain.toml`, Qt 6.12.0, FFmpeg, ExifTool, and the pinned `just`, `cargo-deny`, `cargo-machete`,
`cargo-modules` (the module-graph check), ruff and markdownlint. It is the only Linux environment this repository uses:
the checks, the callgrind benches, the fuzzer and the eval's Linux parts all run in it. Docker Desktop must be running.

From the repository root, through the Justfile:

| Command | What it does |
|---|---|
| `just linux-build` | builds the images: `pg-dev` (the checks), `pg-tools` (the benches and the fuzzer) and `pg-eval` (the eval's Linux parts) |
| `just check-linux` | the Linux job of CI, step by step (`docker/check.py`), on the working tree |
| `just check-linux-head` | the same, on the committed HEAD (a `git archive` of it), uncommitted changes left out |
| `just perf` | the callgrind benches of HEAD, gated against `docs/perf/baseline.json` (±1%); `just perf --update` after a change that should move the counts |
| `just fuzz-smoke 60` | each fuzz target for 60 seconds (`fuzz/`); a crash leaves its input in `fuzz/artifacts/` |
| `just linux-eval SCRIPT ARGS` | an eval script under `eval/` in `pg-eval`, on the working tree: the JPEG comparisons (`eval/jpeg-decoders/linux_build.py`, `linux.py`, `harness_linux.py`, `restart.py`, `encoders.py`) |
| `just linux-gallery OUT` | a headless Linux `gallery` (release, no Qt) of the working tree, copied into the host folder `OUT`: a binary to run elsewhere (WSL only runs binaries) |
| `just linux-shell` | a shell in `pg-dev`, on the working tree |

`tools/linux.py` is what those recipes run. It builds an image when it is missing, passes the versions in, and mounts
the repository. The images are built with the versions the Justfile pins (`just_version`, `ruff_version`,
`cargo_deny_version`, `cargo_machete_version`, `cargo_modules_version`, `cargo_fuzz_version`, `qt_linux_version`,
`aqtinstall_commit`) and the gungraun version in `Cargo.lock`, so each version is written once. Build the image by hand
only through `linux-build`: without the build arguments the Dockerfile fails on purpose.

Each command runs in a new container, removed when the command ends, so its output is in that run's logs. The compiled
state is in Docker's named volumes, which outlive the containers, so a run reuses the last run's build.

`docker/check.py` runs the steps of CI's Linux job in CI's order and stops at the first failure. It also fails when a
test prints `skipped:` (the test checked nothing).

The bench runs (`just perf`, `just linux-bench`, and `just linux-eval`, whose JPEG comparison runs callgrind) start their containers with `--security-opt seccomp=unconfined`. Gungraun
runs valgrind under `setarch -R` (ASLR off, as the baseline was measured), and Docker's default seccomp profile refuses
the personality call that needs. The checks and the fuzzer keep the default profile.

## What the container runs as

The checks run as the user `pg` (UID 1001, the GitHub runner's user), not root. Root ignores file permissions, so the
tests that need a file the process can't read or write skip as root and pass without checking anything. `pg` is what
makes them check.

The `/proc` device test (`crates/platform/src/dir.rs`) needs no Docker flag: it compares each entry with its own
device, so masked paths under `/proc` are correct, not a failure.

## Volumes and caches

- Build output (`/cargo-target`) and the registry (`/home/pg/.cargo/registry`) are Docker named volumes: `pg-cargo-target`
  and `pg-cargo-registry`. The repository's own `target` folder is not touched. The first run compiles everything; later
  runs reuse it. To start from nothing: `docker volume rm pg-cargo-target`.
- The benches keep their target and their data in `pg-bench`. `just perf` reads the bench data from the folder
  `PG_BENCH_DATA_SOURCE` names in `.env`, mounted read-only (`docs/PROFILING.md`).
- A volume made by another user cannot be written by `pg`. The fix is to hand it to `pg` once:
  `docker run --rm --user root --entrypoint chown -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/home/pg/.cargo/registry pg-dev -R pg:pg /cargo-target /home/pg/.cargo/registry`
- The eval's sources, builds, inputs and clone are in `pg-eval`, mounted at `/eval`. Two folders from `.env` are mounted
  when set: `JPEG_SRC_DIR` (the JPEG comparison's photos, read-only) and `PG_JPEG_ENC_DIR` (the encoder comparison's work
  folder, which Windows' `gpu_encoders.py` writes into too).
- The ANN bake-off's CPU worker (`eval/ann/cpu_worker.py`) runs in `pg-eval` through `tools/linux.py ann-worker`, which
  `eval/ann/methods.py` calls with a job file: the folders the job names are mounted (its output folder read-write, its
  inputs read-only), and the worker reads a copy of the job with the container's paths.
- CI sets `PG_LINUX_CACHE` to a folder it restores and saves (`actions/cache`), in place of the named volumes.
- Every host folder mounted must be one Docker Desktop shares (Settings > Resources > File sharing, its
  `FilesharingDirectories`). For any other folder, `docker run` waits, printing nothing, until the "share this
  folder?" prompt in Docker Desktop's window is answered. `tools/linux.py` checks each host folder against that list
  before it starts a container, and fails at once on one that isn't shared. The folders the recipes mount: the checkout,
  `PG_BENCH_DATA_SOURCE`, `JPEG_SRC_DIR`, `PG_JPEG_ENC_DIR`, and the folders of an ANN job (by default under
  `PG_CACHE_DIR`'s `ann`).

## Scope

The container runs the Linux checks, benches, fuzzer and the eval's Linux parts. The Windows job (`just lint`, `just test`, the Qt desktop app,
`just shots`) still runs on Windows. The container does not reproduce a Windows drive or a WSL drvfs mount; the one
check that needs one (S-17.7's renumbering, `docs/S17_7_PLAN.md`) is the exception that uses WSL, and only to run binaries.
