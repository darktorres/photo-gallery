"""What the repository's Python tools share: the repository's folder, the `.env` file (the Justfile loads it for recipes; a direct
run reads it here), the lint tools at CI's pins (in `.venv` and `tools/lint`, else on PATH) and running a command. Standard
library only, the same on every OS.
"""

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

if sys.version_info < (3, 12):  # noqa: UP036 (the target is the floor; this guard is for an older Python)
    sys.exit(
        f"these tools need Python 3.12 or later (this is {sys.version.split()[0]}): install it, and set PG_PYTHON in .env "
        "to its path"
    )

REPO = Path(__file__).resolve().parent.parent
WINDOWS = os.name == "nt"
_ENV_LINE = re.compile(r"^\s*([^#=\s][^=]*?)\s*=\s*(.*?)\s*$")


def load_env_file() -> None:
    """Sets the keys of the repository's `.env` that the environment does not set (a variable already set wins). The
    container (docker/Dockerfile sets PG_CONTAINER) loads none: its `.env` is this machine's Windows paths, and the
    container has its own environment."""
    env_file = REPO / ".env"
    if os.environ.get("PG_CONTAINER") or not env_file.is_file():
        return
    for line in env_file.read_text(encoding="utf-8").splitlines():
        match = _ENV_LINE.match(line)
        if match and match.group(1) not in os.environ:
            os.environ[match.group(1)] = env_value(match.group(2))


def env_value(raw: str) -> str:
    """A `.env` value as `just`'s dotenv loader reads it: without the quotes around it (`"C:/Program Files/LLVM"`, a value
    with a space, is written quoted), so a script run directly sees what it sees under `just`."""
    if len(raw) >= 2 and raw[0] == raw[-1] and raw[0] in "\"'":
        return raw[1:-1]
    return raw


def venv_exe(name: str) -> Path:
    """A program of the repository's `.venv` (its `Scripts` folder on Windows, `bin` elsewhere)."""
    folder = "Scripts" if WINDOWS else "bin"
    return REPO / ".venv" / folder / (name + ".exe" if WINDOWS else name)


LINT_DIR = REPO / "tools" / "lint"


def lint_pins() -> dict[str, str]:
    """The versions CI installs, from the lockfiles: ruff (requirements.txt) and markdownlint-cli (package-lock.json)."""
    pins = {}
    for line in (LINT_DIR / "requirements.txt").read_text(encoding="utf-8").splitlines():
        match = re.match(r"^ruff==(\S+)", line)
        if match:
            pins["ruff"] = match.group(1)
    lock = json.loads((LINT_DIR / "package-lock.json").read_text(encoding="utf-8"))
    pins["markdownlint"] = lock["packages"]["node_modules/markdownlint-cli"]["version"]
    if "ruff" not in pins:
        sys.exit("tools/lint/requirements.txt pins no ruff version")
    return pins


def lint_tool(name: str) -> str | None:
    """The lint tool `name`: ruff from `.venv`, markdownlint from `tools/lint`'s node_modules, else whichever is on PATH."""
    if name == "ruff" and venv_exe("ruff").is_file():
        return str(venv_exe("ruff"))
    if name == "markdownlint":
        local = LINT_DIR / "node_modules" / ".bin" / ("markdownlint.cmd" if WINDOWS else "markdownlint")
        if local.is_file():
            return str(local)
    return shutil.which(name)


def import_msvc_environment(build_tools: Path, toolset: str | None = None) -> str:
    """Sets this process's environment for MSVC's command line (vcvars64.bat, with the toolset when one is given) and returns
    the path of its cl.exe. With a toolset, cl.exe must be that toolset's. Windows only. A failure ends this program."""
    vcvars = build_tools / "VC" / "Auxiliary" / "Build" / "vcvars64.bat"
    if not vcvars.is_file():
        sys.exit(f"vcvars64.bat not found: {vcvars} (Visual Studio Build Tools, see the pins)")
    pin = f" -vcvars_ver={toolset}" if toolset else ""
    # A string, not a list: a list would have its quotes escaped with backslashes, which cmd.exe does not read.
    dump = subprocess.run(
        f'cmd.exe /d /c ""{vcvars}"{pin} >nul 2>nul && set"',
        capture_output=True,
        text=True,
        check=False,
    )
    if dump.returncode != 0:
        sys.exit(f"vcvars64.bat failed (exit {dump.returncode})")
    for line in dump.stdout.splitlines():
        key, sep, value = line.partition("=")
        if sep and key:
            os.environ[key] = value
    cl = shutil.which("cl.exe")
    if cl is None:
        sys.exit("cl.exe is not on PATH after vcvars64.bat")
    if toolset and f"\\{toolset}\\" not in cl:
        sys.exit(f"cl.exe is not toolset {toolset}: {cl}")
    return cl


def run(cmd: list[str], env: dict[str, str] | None = None, quiet: bool = False, cwd: Path = REPO) -> None:
    """Runs `cmd` in `cwd` (the repository by default), printing it first; a failing command ends this program with its exit
    status. `quiet` discards its output (for a check whose output is only data, such as `cargo metadata`)."""
    print("+ " + " ".join(cmd), flush=True)
    result = subprocess.run(cmd, cwd=cwd, env=env, check=False, stdout=subprocess.DEVNULL if quiet else None)
    if result.returncode != 0:
        sys.exit(result.returncode)
