#!/usr/bin/env python3
"""The Linux job of CI (.github/workflows/ci.yml), step by step, in the container's /work (the repository). The lint steps
are tools/lint.py's (the same list `just lint` runs); the tests run here. Each step stops the run on failure, as in CI.
Run it through the launcher, from the repository root: `just check-linux` (tools/linux.py check), which builds the image
and mounts the working tree here.

A test that can't check what it is for returns early and passes, after it prints `skipped: <reason>` (pg_testkit::skip) or
`<tool> not found: skipped` (missing_tool). The container runs as `pg`, so none of the permission-based tests should skip:
any skip fails this step, since such a test checked nothing.
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SKIPPED = re.compile(r"skipped: |: skipped$")


def main() -> None:
    print("\n== Lint (tools/lint.py: formatting, clippy, docs, lockfiles, supply chain, Python and Markdown)", flush=True)
    lint = [sys.executable, str(REPO / "tools" / "lint.py")]
    print("+ " + " ".join(lint), flush=True)
    result = subprocess.run(lint, cwd=REPO, check=False)
    if result.returncode != 0:
        sys.exit(result.returncode)

    print("\n== Tests (the whole workspace)", flush=True)
    # --show-output keeps the printed lines of passing tests too (libtest captures them otherwise), so the skip lines reach
    # the check below.
    command = ["cargo", "test", "--workspace", "--locked", "--", "--show-output"]
    print("+ " + " ".join(command), flush=True)
    process = subprocess.Popen(
        command, cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace"
    )
    lines = []
    assert process.stdout is not None  # the pipe is set above
    for line in process.stdout:
        print(line, end="", flush=True)
        lines.append(line)
    code = process.wait()
    if code != 0:
        sys.exit(code)
    skips = [line.rstrip() for line in lines if SKIPPED.search(line)]
    if skips:
        print("\nFAILED: tests skipped, so they passed without checking anything:", file=sys.stderr)
        print("\n".join(skips), file=sys.stderr)
        sys.exit(1)

    print("\nAll Linux checks passed.")


if __name__ == "__main__":
    main()
