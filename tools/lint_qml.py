#!/usr/bin/env python3
"""The QML's lint (`just lint-qml` on Linux and macOS), failing on any warning: qmllint's, and the QML compiler's. Every
binding and function must compile to C++ (qmlcachegen, at build time), none left to the interpreter. qmlcachegen falls
back to bytecode quietly, and says why only with --verbose; qmllint's own compiler pass (--compiler) misses much of what
it rejects, so qmlcachegen itself is run here as the build runs it. Run this after a build: it reads the types of the
pg.gallery module (the Rust QObjects) from the last build's plugin.qmltypes.

The module is laid out in target/qmllint/pg/gallery as the build lays it out (vendor/README.md, qt-build-utils): its
qmldir, its qmltypes and, beside them, copies of the QML as the qmldir lists them (qml/Main.qml, ...), plus a resource file
mapping the module's resource paths to them, which qmlcachegen resolves the module through. The copies are of the sources
as they are now, so the QML can be linted again without a rebuild. qmllint matches a file to its qmldir entry (Ratings is a
singleton only through it) by the path it was given, so the copies are linted.

Run with the same command on every OS (`just lint-qml`). The qmllint and qmlcachegen calls, and the warnings, are printed as
crates/desktop/qml paths. The Qt kit is QT_DIR (from the environment or .env); its tools are read from its bin folder only,
never from PATH, so another Qt on PATH cannot lint against the wrong types.
"""

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
QML_SOURCES = REPO / "crates" / "desktop" / "qml"
WARNING = re.compile(r"^(?:Warning|Error): (.+?):(\d+):(\d+): (.*)$")


def load_env_file() -> None:
    """Keys of the repo's .env that the environment does not set (the Justfile loads it for recipes; a direct run does here)."""
    env_file = REPO / ".env"
    if not env_file.is_file():
        return
    for line in env_file.read_text(encoding="utf-8").splitlines():
        match = re.match(r"^\s*([^#=\s][^=]*?)\s*=\s*(.*?)\s*$", line)
        if match and match.group(1) not in os.environ:
            os.environ[match.group(1)] = match.group(2)


def qt_tool(qt: Path, name: str) -> str:
    """A Qt tool from the kit, never from PATH: its bin folder, then libexec (Linux keeps qmlcachegen there). Windows names
    it with .exe."""
    exe = name + (".exe" if os.name == "nt" else "")
    for folder in ("bin", "libexec"):
        tool = qt / folder / exe
        if tool.is_file():
            return str(tool)
    sys.exit(f"{exe} is not in {qt / 'bin'} or {qt / 'libexec'}: QT_DIR must be the Qt kit folder (the one with bin and lib)")


def newest_qmltypes() -> Path:
    """The build script's output: build/pg-desktop-<hash>/ with stable cargo, build/pg-desktop/<hash>/ with the pinned nightly."""
    patterns = [
        "target/*/build/pg-desktop-*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes",
        "target/*/build/pg-desktop/*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes",
    ]
    found = [p for pattern in patterns for p in REPO.glob(pattern)]
    if not found:
        sys.exit("No pg.gallery module in target/: build the app first (just build).")
    return max(found, key=lambda p: p.stat().st_mtime)


def main() -> int:
    load_env_file()
    qt_dir = os.environ.get("QT_DIR")
    if not qt_dir:
        sys.exit("QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in .env.")
    qt = Path(qt_dir)
    qmllint = qt_tool(qt, "qmllint")
    qmlcachegen = qt_tool(qt, "qmlcachegen")

    types = newest_qmltypes()
    imports = REPO / "target" / "qmllint"
    module = imports / "pg" / "gallery"
    if imports.exists():
        shutil.rmtree(imports)
    (module / "qml").mkdir(parents=True)
    shutil.copy(types, module / types.name)
    shutil.copy(types.parent / "qmldir", module / "qmldir")
    names = sorted(p.name for p in QML_SOURCES.glob("*.qml"))
    for name in names:
        shutil.copy(QML_SOURCES / name, module / "qml" / name)

    entries = [f'<file alias="qmldir">{module / "qmldir"}</file>']
    entries += [f'<file alias="qml/{name}">{module / "qml" / name}</file>' for name in names]
    qrc = module / "module.qrc"
    qrc.write_text(f'<RCC><qresource prefix="/qt/qml/pg/gallery">{"".join(entries)}</qresource></RCC>', encoding="utf-8")

    warnings: list[str] = []

    # Paths as forward slashes, on every OS, so the copies' folder is matched the same way on Windows.
    module_qml = (module / "qml").as_posix() + "/"

    def add_warning(path: str, line: str, column: str, message: str) -> None:
        name = Path(path).as_posix().replace(module_qml, "crates/desktop/qml/")
        warnings.append(f"{name}:{line}:{column}: {message}")

    report = subprocess.run(
        [qmllint, "-I", str(imports), "--json", "-"] + [str(module / "qml" / name) for name in names],
        capture_output=True,
        text=True,
        check=False,
    )
    # qmllint writes its JSON report to stdout; a report that does not parse is a failure of the tool itself.
    try:
        data = json.loads(report.stdout)
    except json.JSONDecodeError:
        sys.exit(f"qmllint printed no JSON report (exit {report.returncode}):\n{report.stderr}")
    for file in data.get("files", []):
        for w in file.get("warnings", []):
            add_warning(file["filename"], str(w["line"]), str(w["column"]), f"{w['message']} [{w['id']}]")

    # qmlcachegen says why it left something to the interpreter (--verbose), and lists what it compiled and what it didn't
    # (--dump-aot-stats, into <output>.aotstats): the verdict, should a reason go unsaid.
    compiled = 0
    functions = 0
    for name in names:
        out = imports / f"{name}.cpp"
        run = subprocess.run(
            [
                qmlcachegen,
                "-i",
                str(module / "qmldir"),
                "--resource",
                str(qrc),
                "--resource-path",
                f"/qt/qml/pg/gallery/qml/{name}",
                "--verbose",
                "--dump-aot-stats",
                "--module-id",
                "pg.gallery",
                "-o",
                str(out),
                str(module / "qml" / name),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        # Its findings come on stderr, which isn't an error here; a failed run is.
        log = (run.stdout + run.stderr).splitlines()
        if run.returncode != 0:
            sys.exit(f"qmlcachegen failed on {name}:\n" + "\n".join(log))
        # Each finding is one "Warning:"/"Error:" line (the source line and a caret under it follow).
        for line in log:
            match = WARNING.match(line)
            if match:
                add_warning(match.group(1), match.group(2), match.group(3), match.group(4))
        stats = json.loads(Path(f"{out}.aotstats").read_text(encoding="utf-8"))
        # The stats list each module's files, and each file's entries (PowerShell flattened these for the Windows lint).
        entries = [e for module in stats["modules"] for mf in module["moduleFiles"] for e in mf["entries"]]
        for entry in entries:
            functions += 1
            if entry["codegenResult"] == 0:
                compiled += 1
                continue
            add_warning(
                str(module / "qml" / name),
                str(entry["line"]),
                str(entry["column"]),
                f"{entry['functionName']} is left to the interpreter: {entry['message']} [aot]",
            )

    # Both tools report some findings alike.
    unique = list(dict.fromkeys(warnings))
    for line in unique:
        print(line)
    if unique:
        print(f"{len(unique)} QML warning(s); {compiled} of {functions} bindings and functions compiled to C++.")
        return 1
    print(f"QML: no warnings; all {functions} bindings and functions compiled to C++ ({len(names)} files).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
