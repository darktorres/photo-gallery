# The Linux environment of CI's Linux job (.github/workflows/ci.yml), for building, checking, benchmarking and fuzzing this
# repository. Built and run by tools/linux.py, which passes the versions below from the Justfile, so
# each version is written once. Three targets: `base` runs the checks (docker/check.py); `tools` adds the callgrind
# benches and the fuzzer on top of it; `eval` adds the eval's Linux parts (eval/ann's CPU worker, the JPEG comparisons).
# The base image is pinned by digest (the index: `docker buildx imagetools inspect ubuntu:26.04`), so a rebuild does not
# change it under the pinned versions. Moving to a newer one is a commit, with its digest.
# uv's own image, for uv's binary: the base stage copies it. Docker needs the version in a global ARG to name the image.
# The default is the Justfile's uv_version: a test (tools/test_ci_files.py) fails when the two differ, so a bump changes both.
ARG UV_VERSION=0.13.10
FROM ghcr.io/astral-sh/uv:${UV_VERSION} AS uv

FROM ubuntu:26.04@sha256:d66b366dac61b575bee2aef07e9c26e8070096fbe84a3d4d3eb0da069b5d76db AS base

ARG JUST_VERSION
ARG CARGO_DENY_VERSION
ARG CARGO_MACHETE_VERSION
ARG CARGO_MODULES_VERSION
ARG SCCACHE_VERSION
ARG AQTINSTALL_COMMIT
ARG QT_VERSION
ARG EXIFTOOL_VERSION
ARG EXIFTOOL_SOURCE_URL
ARG EXIFTOOL_SOURCE_SHA256
ARG NODE_VERSION
ARG NODE_LINUX_X64_SHA256

# Everything the checks use lives outside /root (Ubuntu makes it mode 0700): the checks run as the user `pg`, not as
# root, because root ignores file permissions and the permission-based tests (testkit's `hold`) skip as root.
ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    PG_REQUIRE_TOOLS=1 \
    UV_PYTHON_INSTALL_DIR=/opt/uv-python \
    UV_PROJECT_ENVIRONMENT=/opt/pg-venv \
    QT_QPA_PLATFORM=offscreen \
    RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
    CARGO_HOME=/home/pg/.cargo \
    RUSTUP_HOME=/home/pg/.rustup \
    CARGO_TARGET_DIR=/cargo-target \
    PG_FFMPEG=/usr/bin/ffmpeg \
    LIBCLANG_PATH=/opt/libclang \
    QMAKE=/opt/Qt/${QT_VERSION}/gcc_64/bin/qmake \
    LD_LIBRARY_PATH=/opt/Qt/${QT_VERSION}/gcc_64/lib \
    PATH=/opt/node/bin:/opt/lint/bin:/opt/lint-src/node_modules/.bin:/home/pg/.cargo/bin:/opt/exiftool:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# The system packages: the list CI installs (apt-packages.txt, which has git, the aqtinstall install below being from a git
# URL), the mold linker (CI's Linux job links with it), the venv module, its own package on Ubuntu, and xz (to unpack Node,
# below). Node itself comes from nodejs.org, not apt. FFmpeg's headers come from its development packages, which pkg-config
# finds (crates/video/build.rs).
COPY docker/apt-packages.txt /opt/pg/apt-packages.txt
RUN apt-get update \
 && grep -v '^#' /opt/pg/apt-packages.txt | grep -v '^$' | xargs apt-get install -y --no-install-recommends \
      ca-certificates curl libatomic1 mold python3-venv xz-utils \
 && rm -rf /var/lib/apt/lists/*

# ExifTool at the Justfile's pinned release (the source archive, checked by SHA-256; tools/install_exiftool.py), in /opt, which
# every user reads. Not an apt package: apt would give whatever release the image carries, not the one the tests are pinned to.
COPY tools/install_exiftool.py /opt/pg/install_exiftool.py
RUN python3 /opt/pg/install_exiftool.py --url "${EXIFTOOL_SOURCE_URL}" --sha256 "${EXIFTOOL_SOURCE_SHA256}" \
      --version "${EXIFTOOL_VERSION}" --prefix /opt/exiftool

# libclang for bindgen: the newest LLVM the image has, at a fixed path (LIBCLANG_PATH), as CI finds it.
RUN ln -sfn "$(find /usr/lib -maxdepth 2 -path '/usr/lib/llvm-*/lib' | sort -V | tail -1)" /opt/libclang

# The user the checks run as: UID 1001, the GitHub runner's user, so a checkout the runner made is writable by it.
RUN useradd --uid 1001 --create-home --shell /bin/bash pg

# Git reads the repository from the bind mount, which belongs to the host user. System scope, so every user gets it.
RUN git config --system --add safe.directory '*'

# uv, pinned in the Justfile (uv_version): it runs every recipe's Python (`uv run`, from the repository's pyproject.toml and
# uv.lock), installs the lint group, and installs the Qt installer below. Copied from uv's own image, so nothing is installed
# with pip to get it.
COPY --from=uv /uv /usr/local/bin/uv

# Qt (the Justfile's qt_version) for Linux, with qtmultimedia: aqtinstall as a uv tool, from the commit that CI's Windows job also
# pins. Installed as root into /opt, which every user reads; the tool's binary goes to /usr/local/bin.
RUN UV_TOOL_DIR=/opt/uv-tools UV_TOOL_BIN_DIR=/usr/local/bin uv tool install "aqtinstall @ git+https://github.com/miurahr/aqtinstall.git@${AQTINSTALL_COMMIT}" \
 && aqt install-qt --outputdir /opt/Qt linux desktop ${QT_VERSION} linux_gcc_64 --modules qtmultimedia

# Node (the Justfile's node_version) from nodejs.org, checked by SHA-256 before it is unpacked: npm, and markdownlint runs on it.
# Installed as root into /opt/node, which every user reads; its bin is first on PATH.
RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" -o /tmp/node.tar.xz \
 && echo "${NODE_LINUX_X64_SHA256}  /tmp/node.tar.xz" | sha256sum -c - \
 && tar -xJf /tmp/node.tar.xz -C /opt \
 && mv "/opt/node-v${NODE_VERSION}-linux-x64" /opt/node \
 && rm /tmp/node.tar.xz

# The Python and Markdown lint tools: ruff in the lint group of pyproject.toml (uv.lock holds its hashes), and markdownlint-cli
# by its lockfile (npm ci). Installed as root, read by every user. The interpreter is uv's, the version .python-version names.
COPY pyproject.toml uv.lock .python-version /opt/lint-src/
COPY tools/lint/package.json tools/lint/package-lock.json /opt/lint-src/
RUN uv python install "$(cat /opt/lint-src/.python-version)" \
 && cd /opt/lint-src \
 && UV_PROJECT_ENVIRONMENT=/opt/lint uv sync --frozen --only-group lint \
 && npm ci --prefix /opt/lint-src --no-audit --no-fund

# The directories the volumes are mounted on, owned by `pg`: a new named volume starts with its directory's owner.
RUN install -d -o pg -g pg /cargo-target /home/pg/.cargo /home/pg/.cargo/registry /home/pg/.cargo/git /bench /sccache /opt/pg-venv

# The toolchain file, and the rustup step that installs what it names (the pinned nightly, its components and target).
COPY rust-toolchain.toml /opt/pg/rust-toolchain.toml

USER pg

# rustup's installer, checked against the pin in the Justfile (rustup_init_sha256) before it runs.
ARG RUSTUP_INIT_URL=https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init
ARG RUSTUP_INIT_SHA256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
RUN curl -sSf -o /tmp/rustup-init "${RUSTUP_INIT_URL}" \
 && echo "${RUSTUP_INIT_SHA256}  /tmp/rustup-init" | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && /tmp/rustup-init -y --profile minimal \
 && rm /tmp/rustup-init \
 && . "$HOME/.cargo/env" \
 && cd /opt/pg && rustup show

# The tools the Justfile pins (just, cargo-deny, cargo-machete, sccache) and the module-graph tool of the cycle check
# (tools/modgraph). Installed with the default toolchain, as CI's install step does for just. sccache without its default
# features: only its local disk cache is used, not the remote storage backends.
RUN cargo install --locked just --version ${JUST_VERSION} \
 && cargo install --locked cargo-deny --version ${CARGO_DENY_VERSION} \
 && cargo install --locked cargo-machete --version ${CARGO_MACHETE_VERSION} \
 && cargo install --locked cargo-modules --version ${CARGO_MODULES_VERSION} \
 && cargo install --locked sccache --version ${SCCACHE_VERSION} --no-default-features

# Every rustc the checks run goes through sccache, whose cache is the pg-sccache volume at /sccache (tools/linux.py). Set after
# the installs above, which run before sccache exists. A container's sccache server ends with the container; its cache stays.
ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache \
    SCCACHE_CACHE_SIZE=30G

# The `.env` in /work describes the Windows machine, so the scripts do not load it here (tools/pgtools.py).
ENV PG_CONTAINER=1

WORKDIR /work
CMD ["python3", "docker/check.py"]

# The callgrind benches and the fuzzer: their packages, and their tools at the versions the Justfile
# pins (cargo-fuzz) and at the version of the gungraun library in Cargo.lock (gungraun-runner, which the launcher reads).
FROM base AS tools

ARG CARGO_FUZZ_VERSION
ARG GUNGRAUN_VERSION

USER root
COPY docker/apt-packages-bench.txt /opt/pg/apt-packages-bench.txt
RUN apt-get update \
 && grep -v '^#' /opt/pg/apt-packages-bench.txt | grep -v '^$' | xargs apt-get install -y --no-install-recommends \
 && rm -rf /var/lib/apt/lists/*

USER pg

# The runner must be the exact version of the gungraun library in Cargo.lock. No --locked: gungraun-runner 0.20.0's
# published lock pins yoke-derive 0.8.3, which was yanked, so cargo picks the non-yanked release.
# RUSTC_WRAPPER is cleared for these installs: through sccache they would leave its cache in the image layer.
RUN RUSTC_WRAPPER= cargo install gungraun-runner --version ${GUNGRAUN_VERSION} \
 && RUSTC_WRAPPER= cargo install --locked cargo-fuzz --version ${CARGO_FUZZ_VERSION}

# The eval's Linux parts: the packages the
# JPEG builds and runs need (apt-packages-eval.txt), and a venv with the Python packages the scripts import: the eval-linux
# group of pyproject.toml, synced from uv.lock. /eval is the pg-eval volume's mount point.
FROM tools AS eval

USER root
COPY docker/apt-packages-eval.txt /opt/pg/apt-packages-eval.txt
RUN apt-get update \
 && grep -v '^#' /opt/pg/apt-packages-eval.txt | grep -v '^$' | xargs apt-get install -y --no-install-recommends \
 && rm -rf /var/lib/apt/lists/*
RUN cd /opt/lint-src \
 && UV_PROJECT_ENVIRONMENT=/opt/eval uv sync --frozen --only-group eval-linux \
 && install -d -o pg -g pg /eval

USER pg
