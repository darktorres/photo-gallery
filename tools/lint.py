#!/usr/bin/env python3
"""The lint steps, the same on every OS: `just lint`. The clippy and docs steps cover the whole workspace, so on Windows
they also check the code that compiles only there (cfg(windows)). The QML lint is its own recipe (`just lint-qml`,
tools/lint_qml.py), since it reads the build's output. Stops at the first step that fails.

The lint tools (ruff and markdownlint) come from the repository or PATH (tools/pgtools.py); `just lint` checks their
versions first (tools/check_lint_tools.py).
"""

import argparse
import os
import sys

from pgtools import lint_tool, load_env_file, run

# Each step is a command; `ruff` and `markdownlint` are resolved to the tool they name (see `resolved`).
STEPS = [
    ["cargo", "fmt", "--all", "--check"],
    ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--timings", "--", "-D", "warnings"],
    ["cargo", "check", "-p", "pg-model", "-p", "pg-client", "--locked", "--target", "wasm32-unknown-unknown"],
    # The workspaces outside the root (fuzz and eval/jpeg-decoders have their own lockfiles): their lock must not be stale.
    ["cargo", "metadata", "--locked", "--manifest-path", "fuzz/Cargo.toml", "--format-version", "1"],
    ["cargo", "metadata", "--locked", "--manifest-path", "eval/jpeg-decoders/Cargo.toml", "--format-version", "1"],
    ["cargo", "doc", "--workspace", "--no-deps", "--document-private-items", "--locked", "--timings"],
    ["cargo", "deny", "check"],
    ["cargo", "machete", "crates"],
    ["ruff", "format", "--check", "."],
    ["ruff", "check", "."],
    ["markdownlint", "**/*.md"],
]
# The docs step's warnings are errors.
DOC_FLAGS = {"RUSTDOCFLAGS": "-D warnings"}


def resolved(cmd: list[str]) -> list[str]:
    """`cmd` with its lint tool's path in place of the name, which must be found."""
    if cmd[0] not in ("ruff", "markdownlint"):
        return cmd
    exe = lint_tool(cmd[0])
    if exe is None:
        sys.exit(f"{cmd[0]} is not installed: run `just lint-tools-setup`")
    return [exe, *cmd[1:]]


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.parse_args()
    load_env_file()
    for step in STEPS:
        env = {**os.environ, **DOC_FLAGS} if step[:2] == ["cargo", "doc"] else None
        # `cargo metadata` prints the whole graph; the check is only that it resolves.
        run(resolved(step), env, quiet=step[:2] == ["cargo", "metadata"])
    print("lint: every step passed")


if __name__ == "__main__":
    main()
