"""What the repository's Python tools share: the repository's folder, the `.env` file (the Justfile loads it for recipes; a direct
run reads it here), the lint tools at CI's pins (in `.venv` and `tools/lint`, else on PATH) and running a command. Standard
library only, the same on every OS.
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

if sys.version_info < (3, 12):  # noqa: UP036 (the target is the floor; this guard is for an older Python)
    sys.exit(
        f"pgtools: these tools need Python 3.12 or later (this is {sys.version.split()[0]}): install it, and set PG_PYTHON "
        "in .env to its path"
    )

REPO = Path(__file__).resolve().parent.parent
WINDOWS = os.name == "nt"
_ENV_LINE = re.compile(r"^\s*([^#=\s][^=]*?)\s*=\s*(.*?)\s*$")
# The MSVC toolset of the static builds (vcvars64 -vcvars_ver). The PowerShell copies keep their own; a tools test asserts
# they agree.
MSVC_TOOLSET = "14.44.35207"
# The default Visual Studio Build Tools and MSYS2 folders of the static builds. The PowerShell copies keep their own; the
# tools test asserts they agree.
DEFAULT_BUILD_TOOLS = Path(r"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools")
DEFAULT_MSYS = Path(r"C:\tools\msys64")


# The profiling tools' defaults, shared by build.py, profile.py and symbolize.py. profile.ps1 keeps its own copy of the Pin
# default (it cannot import this module). Each raises KeyError when its environment variable is unset: callers check first.
def default_llvm_dir() -> Path:
    """LLVM's default folder, %ProgramFiles%\\LLVM (the PG_LLVM_DIR default)."""
    return Path(os.environ["PROGRAMFILES"]) / "LLVM"


def default_pin_root() -> Path:
    """Intel SDE's Pin kit, %LOCALAPPDATA%\\Programs\\sde\\pinkit (the PIN_ROOT default; pin.exe is in it)."""
    return Path(os.environ["LOCALAPPDATA"]) / "Programs" / "sde" / "pinkit"


# The Qt kit folder under a Qt version: the MSVC kit on Windows, GCC elsewhere (`QT_KIT` is this OS's).
WINDOWS_QT_KIT = "msvc2022_64"
LINUX_QT_KIT = "gcc_64"
QT_KIT = WINDOWS_QT_KIT if WINDOWS else LINUX_QT_KIT


def just_value(name: str) -> str:
    """The value of NAME in the Justfile, read from its text. CI reads the pins before `just` is installed, so this cannot
    run `just --evaluate`. It takes only the one literal line `NAME := "value"`: any other right-hand side, or a second
    assignment, ends the program, so a new kind of assignment is never read wrongly."""
    assign = re.compile(rf"{re.escape(name)}\s*:=")
    lines = [line for line in (REPO / "Justfile").read_text(encoding="utf-8").splitlines() if assign.match(line)]
    if len(lines) != 1:
        sys.exit(f"pgtools: the Justfile must assign {name} exactly once, found {len(lines)}")
    match = re.fullmatch(rf'{re.escape(name)}\s*:=\s*"([^"\\]*)"\s*(?:#.*)?', lines[0])
    if not match:
        sys.exit(f"pgtools: the Justfile's {name} is not a plain double-quoted literal: {lines[0]!r}")
    return match.group(1)


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


def default_python() -> str:
    """The Python the Justfile's `py` uses when PG_PYTHON is not set: `python` on Windows, `python3` elsewhere."""
    return "python" if WINDOWS else "python3"


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
        sys.exit("pgtools: tools/lint/requirements.txt pins no ruff version")
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
        sys.exit(f"pgtools: vcvars64.bat not found: {vcvars} (Visual Studio Build Tools, see the pins)")
    pin = f" -vcvars_ver={toolset}" if toolset else ""
    # A string, not a list: a list would have its quotes escaped with backslashes, which cmd.exe does not read.
    dump = subprocess.run(
        f'cmd.exe /d /c ""{vcvars}"{pin} >nul 2>nul && set"',
        capture_output=True,
        text=True,
        check=False,
    )
    if dump.returncode != 0:
        sys.exit(f"pgtools: vcvars64.bat failed (exit {dump.returncode})")
    for line in dump.stdout.splitlines():
        key, sep, value = line.partition("=")
        if sep and key:
            os.environ[key] = value
    cl = shutil.which("cl.exe")
    if cl is None:
        sys.exit("pgtools: cl.exe is not on PATH after vcvars64.bat")
    if toolset and f"\\{toolset}\\" not in cl:
        sys.exit(f"pgtools: cl.exe is not toolset {toolset}: {cl}")
    return cl


def not_set(name: str, how: str) -> NoReturn:
    """Ends the program: the environment variable `name` is not set. `how` says what to give it, and where (the environment
    or the repository's `.env`). Every tool words this one message the same way."""
    sys.exit(f"pgtools: {name} is not set: {how}")


# What PG_QT_DIR is set to, for `not_set`: the words every tool that needs the Qt kit gives. The PowerShell script that is
# kept (profile.ps1) repeats them, since it cannot import this module.
QT_DIR_HOW = "point it at the Qt kit folder (the one with bin and lib), in the environment or in .env"


def checked(what: str, command: list[str], log: Path | None = None, cwd: Path | None = None) -> None:
    """Runs `command`, its output in `log` when given; a failure ends this program."""
    if log is None:
        result = subprocess.run(command, cwd=cwd, check=False)
    else:
        with log.open("w", encoding="utf-8") as file:
            result = subprocess.run(command, cwd=cwd, stdout=file, stderr=subprocess.STDOUT, check=False)
    if result.returncode != 0:
        sys.exit(f"pgtools: {what} failed (exit {result.returncode})")


def sha256(path: Path) -> str:
    """The file's SHA-256 in upper case, as the pins and the manifests write it."""
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def run(cmd: list[str], env: dict[str, str] | None = None, quiet: bool = False, cwd: Path = REPO) -> None:
    """Runs `cmd` in `cwd` (the repository by default), printing it first; a failing command ends this program with its exit
    status. `quiet` discards its output (for a check whose output is only data, such as `cargo metadata`)."""
    print("+ " + " ".join(cmd), flush=True)
    result = subprocess.run(cmd, cwd=cwd, env=env, check=False, stdout=subprocess.DEVNULL if quiet else None)
    if result.returncode != 0:
        sys.exit(result.returncode)
