# The Linux build and checks of this repository, in a container: the environment of CI's Linux job
# (.github/workflows/ci.yml), on the toolchain rust-toolchain.toml names. Build from the repository root:
#   docker build -f docker/Dockerfile -t pg-dev .
# Run the checks on the working tree (docker/README.md has the full command).
FROM ubuntu:26.04

# Everything the checks use lives outside /root (Ubuntu makes it mode 0700), because the checks run as the user `pg`,
# not as root: the permission-based tests (testkit's `hold`) skip as root, since root ignores file permissions.
ENV DEBIAN_FRONTEND=noninteractive \
    PG_REQUIRE_TOOLS=1 \
    QT_QPA_PLATFORM=offscreen \
    RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
    CARGO_HOME=/home/pg/.cargo \
    RUSTUP_HOME=/home/pg/.rustup \
    CARGO_TARGET_DIR=/cargo-target \
    QMAKE=/opt/Qt/6.12.0/gcc_64/bin/qmake \
    LD_LIBRARY_PATH=/opt/Qt/6.12.0/gcc_64/lib \
    FFMPEG_DIR=/home/pg/.local/share/pg-bench/ffmpeg \
    PATH=/opt/lint/bin:/home/pg/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# curl and ca-certificates first: the setup script fetches rustup with them. nodejs and npm are for markdownlint. git is
# for the aqtinstall pip install below (from a git URL). python3-venv is the venv module, its own package on Ubuntu.
# sudo is for the setup script's apt calls when it runs as `pg`, as CI's runner user does.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git nodejs npm python3-venv sudo \
 && rm -rf /var/lib/apt/lists/*

# The user the checks run as. Its UID is not 1000, which the Ubuntu base image gives its own `ubuntu` user.
RUN useradd --uid 1001 --create-home --shell /bin/bash pg \
 && echo 'pg ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/pg \
 && chmod 0440 /etc/sudoers.d/pg

# Git reads the repository from the bind mount, which belongs to the host user. System scope, so every user gets it.
RUN git config --system --add safe.directory '*'

# The setup script and the toolchain file. `rustup show` in the same folder as the file installs what it names.
COPY tools/profiling /opt/pg/tools/profiling
COPY rust-toolchain.toml /opt/pg/rust-toolchain.toml

# Qt 6.12.0 for Linux, with qtmultimedia, from the same aqtinstall commit as CI's jurplel/install-qt-action. Installed as
# root into /opt, which every user reads.
RUN python3 -m venv /opt/aqt \
 && /opt/aqt/bin/pip install --quiet "git+https://github.com/miurahr/aqtinstall.git@076e1659807d0b362a3ed684d54c2e9c775eb9c7" \
 && /opt/aqt/bin/aqt install-qt --outputdir /opt/Qt linux desktop 6.12.0 linux_gcc_64 --modules qtmultimedia

# The Python and Markdown lint tools, at the versions CI and the Justfile pin. Installed as root, read by every user.
RUN python3 -m venv /opt/lint \
 && /opt/lint/bin/pip install --quiet ruff==0.17.3 \
 && npm install --global markdownlint-cli@0.50.0

# The build output folder, owned by `pg`. A named volume mounted here starts with this folder's owner.
RUN install -d -o pg -g pg /cargo-target

USER pg

# The system packages, the mold linker and the FFmpeg prefix for ffmpeg-sys-next: the script CI runs for its Linux job
# (`wsl-setup.sh --ci`), as the `pg` user. It reads apt-packages.txt from the copy here, so the package list is the same
# one CI installs. Its rustup and its FFmpeg prefix go to $HOME, which is /home/pg, the CARGO_HOME and FFMPEG_DIR above.
RUN HOME=/home/pg bash /opt/pg/tools/profiling/wsl-setup.sh --ci \
 && sudo rm -rf /var/lib/apt/lists/*

# The pinned nightly from rust-toolchain.toml, with clippy, rustfmt and the wasm32 target.
RUN cd /opt/pg && rustup show

# The tools CI installs at the versions the Justfile pins (just, cargo-deny, cargo-machete).
RUN cargo install --locked just --version 1.59.0 \
 && cargo install --locked cargo-deny --version 0.20.2 \
 && cargo install --locked cargo-machete --version 0.9.2

# The module-graph tool for the cycle check (tools/modgraph, docs/MODULARITY_PLAN.md M-0), at the version the plan
# pins. It goes last so that the layers above stay cached when it changes.
RUN cargo install --locked cargo-modules --version 0.28.0

COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/pg-entrypoint

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/pg-entrypoint"]
CMD ["bash", "docker/check.sh"]
