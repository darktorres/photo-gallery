#!/usr/bin/env python3
"""The lint tools at CI's versions. The Python and Node packages are pinned in tools/lint (requirements.txt with hashes, and
package-lock.json), and `--install` installs them: ruff into `.venv`, markdownlint into `tools/lint/node_modules` (both
git-ignored, so other projects on the machine keep their own versions; `just lint-tools-setup`). The default mode fails,
with the fix for each, when one of the four tools is not the version CI uses (`just lint` runs it first). cargo-deny and
cargo-machete are pinned in the Justfile and passed here. A tool is looked for in the repository, then on PATH.
"""

import argparse
import shutil
import subprocess
import sys

from pgtools import LINT_DIR, REPO, lint_pins, lint_tool, load_env_file, run, venv_exe


def version_of(exe: str) -> str | None:
    """The last word of `exe --version`, or None when it does not run."""
    try:
        result = subprocess.run([exe, "--version"], capture_output=True, text=True, check=False)
    except OSError:
        return None
    words = (result.stdout + result.stderr).split()
    return words[-1] if result.returncode == 0 and words else None


def install() -> None:
    """Installs the pinned tools: ruff with its hashes into `.venv`, and markdownlint-cli from its lockfile."""
    venv = REPO / ".venv"
    if not venv.is_dir():
        run([sys.executable, "-m", "venv", str(venv)])
    requirements = str(LINT_DIR / "requirements.txt")
    run([str(venv_exe("python")), "-m", "pip", "install", "--quiet", "--require-hashes", "-r", requirements])
    npm = shutil.which("npm")
    if npm is None:
        sys.exit("npm is not on PATH: install Node.js, which markdownlint runs on")
    run([npm, "ci", "--prefix", str(LINT_DIR), "--quiet", "--no-audit", "--no-fund"])


def check(pins: dict[str, tuple[str, str]]) -> None:
    """Fails listing every tool that is missing or not at its pin: `pins` maps each tool to (version, the fix)."""
    problems = []
    for name, (want, fix) in pins.items():
        exe = lint_tool(name)
        if exe is None:
            problems.append(f"{name} is not installed: {fix}")
            continue
        have = version_of(exe)
        if have != want:
            problems.append(f"{name} is {have}, CI uses {want}: {fix}")
    if problems:
        sys.exit("lint tools do not match CI's versions:\n  " + "\n  ".join(problems))
    print("lint tools match CI: " + ", ".join(f"{name} {want}" for name, (want, _) in pins.items()))


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--install", action="store_true", help="install the pinned ruff and markdownlint into the repository")
    parser.add_argument("--cargo-deny", help="cargo-deny's version (checked only)")
    parser.add_argument("--cargo-machete", help="cargo-machete's version (checked only)")
    args = parser.parse_args()
    load_env_file()
    if args.install:
        install()
        return
    if not (args.cargo_deny and args.cargo_machete):
        parser.error("--cargo-deny and --cargo-machete are needed to check the tools")
    pins = lint_pins()
    check(
        {
            "ruff": (pins["ruff"], "just lint-tools-setup"),
            "markdownlint": (pins["markdownlint"], "just lint-tools-setup"),
            "cargo-deny": (args.cargo_deny, f"cargo install --locked cargo-deny --version {args.cargo_deny}"),
            "cargo-machete": (args.cargo_machete, f"cargo install --locked cargo-machete --version {args.cargo_machete}"),
        }
    )


if __name__ == "__main__":
    main()
