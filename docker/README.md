# Building and checking in a container

The container has the Linux environment of CI's Linux job (`.github/workflows/ci.yml`): Ubuntu 26.04, the toolchain
from `rust-toolchain.toml`, Qt 6.12.0, FFmpeg, ExifTool, and the pinned `just`, `cargo-deny`, `cargo-machete`,
`cargo-modules` (the module-graph check), ruff and markdownlint. The repository is mounted from the host, so edits
show up in the container at once.

Build the image once, and again after the Dockerfile changes (from the repository root):

    docker build -f docker/Dockerfile -t pg-dev .

Run the Linux checks on the working tree:

    docker run --rm --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/home/pg/.cargo/registry pg-dev

`docker/check.sh` runs the steps in CI's order and stops at the first failure. For a shell in the same environment:

    docker run --rm -it --security-opt systempaths=unconfined -v "D:/GitHub/photo-gallery:/work" -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/home/pg/.cargo/registry pg-dev bash

`--security-opt systempaths=unconfined` is needed for the platform crate's `/proc` test. Docker masks some `/proc`
paths with bind mounts, which puts those entries on other devices, and the test (`dir::listing_tests`) expects every
entry under `/proc` on procfs's device. CI's runner doesn't mask them, so the flag keeps the container the same.

The checks run as the user `pg` (UID 1001), not root. Root ignores file permissions, so the tests that need a file
the process can't read or write skip as root and pass without checking anything. `check.sh` fails the run when a test
prints `skipped:`.

- Build output goes to the named volume `pg-cargo-target`, not the repository's `target` folder, so the Windows build
  output is not touched. The first run compiles everything; later runs reuse it. To start from nothing, remove the
  volume: `docker volume rm pg-cargo-target`.
- The registry cache is the named volume `pg-cargo-registry`, mounted at `/home/pg/.cargo/registry`.
- A volume made before the user change was written by root, so `pg` cannot write to it. Hand it to `pg` once, which
  keeps the caches:
  `docker run --rm --user root --entrypoint chown -v pg-cargo-target:/cargo-target -v pg-cargo-registry:/home/pg/.cargo/registry pg-dev -R pg:pg /cargo-target /home/pg/.cargo/registry`
- The checks run on Linux only. The Windows job (`just lint`, `just test`, the Qt desktop app) still runs on Windows.
- The container is for this repository. WSL is not used for it.
