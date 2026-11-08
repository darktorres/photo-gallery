# The Linux build and checks of this repository, in a container: the environment of CI's Linux job
# (.github/workflows/ci.yml), on the toolchain rust-toolchain.toml names. Build from the repository root:
#   docker build -f docker/Dockerfile -t pg-dev .
# Run the checks on the working tree (docker/README.md has the full command).
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive \
    PG_REQUIRE_TOOLS=1 \
    QT_QPA_PLATFORM=offscreen \
    RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
    CARGO_TARGET_DIR=/cargo-target \
    QMAKE=/opt/Qt/6.12.0/gcc_64/bin/qmake \
    LD_LIBRARY_PATH=/opt/Qt/6.12.0/gcc_64/lib \
    FFMPEG_DIR=/root/.local/share/pg-bench/ffmpeg \
    PATH=/opt/lint/bin:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# curl and ca-certificates first: the setup script fetches rustup with them. nodejs and npm are for markdownlint.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl nodejs npm \
 && rm -rf /var/lib/apt/lists/*

# The system packages, the mold linker and the FFmpeg prefix for ffmpeg-sys-next: the script CI runs for its Linux job
# (`wsl-setup.sh --ci`). It reads apt-packages.txt from the copy here, so the package list is the same one CI installs.
COPY tools/profiling /opt/pg/tools/profiling
RUN bash /opt/pg/tools/profiling/wsl-setup.sh --ci \
 && rm -rf /var/lib/apt/lists/*

# Git reads the repository from the bind mount, which belongs to the host user.
RUN git config --global --add safe.directory '*'

# The pinned nightly from rust-toolchain.toml, with clippy, rustfmt and the wasm32 target. `rustup show` in the same
# folder as the file installs what it names, so the repository mounted at /work finds it already here.
COPY rust-toolchain.toml /opt/pg/rust-toolchain.toml
RUN cd /opt/pg && rustup show

# The tools CI installs at the versions the Justfile pins (just, cargo-deny, cargo-machete).
RUN cargo install --locked just --version 1.59.0 \
 && cargo install --locked cargo-deny --version 0.20.2 \
 && cargo install --locked cargo-machete --version 0.9.2

# Qt 6.12.0 for Linux, with qtmultimedia, from the same aqtinstall commit as CI's jurplel/install-qt-action. The venv
# module is its own package on Ubuntu.
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3-venv \
 && rm -rf /var/lib/apt/lists/*
RUN python3 -m venv /opt/aqt \
 && /opt/aqt/bin/pip install --quiet "git+https://github.com/miurahr/aqtinstall.git@076e1659807d0b362a3ed684d54c2e9c775eb9c7" \
 && /opt/aqt/bin/aqt install-qt --outputdir /opt/Qt linux desktop 6.12.0 linux_gcc_64 --modules qtmultimedia

# The Python and Markdown lint tools, at the versions CI and the Justfile pin.
RUN python3 -m venv /opt/lint \
 && /opt/lint/bin/pip install --quiet ruff==0.17.3 \
 && npm install --global markdownlint-cli@0.50.0

# The module-graph tool for the cycle check (tools/modgraph, docs/MODULARITY_PLAN.md M-0), at the version the plan
# pins. It goes last so that the Qt and lint layers above stay cached when it changes.
RUN cargo install --locked cargo-modules --version 0.28.0

COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/pg-entrypoint

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/pg-entrypoint"]
CMD ["bash", "docker/check.sh"]
