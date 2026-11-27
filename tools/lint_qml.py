#!/usr/bin/env python3
"""The QML's lint (`just lint-qml`, on every OS), failing on any warning: qmllint's, and the QML compiler's. Every
binding and function must compile to C++ (qmlcachegen, at build time), none left to the interpreter. qmlcachegen falls
back to bytecode quietly, and says why only with --verbose; qmllint's own compiler pass (--compiler) misses much of what
it rejects, so qmlcachegen itself is run here as the build runs it. Run this after a build: it reads the types of the
pg.gallery module (the Rust QObjects) from the last build's plugin.qmltypes.

The module is laid out in target/qmllint/pg/gallery as the build lays it out (qt-build-utils): its
qmldir, its qmltypes and, beside them, copies of the QML as the qmldir lists them (qml/Main.qml, ...), plus a resource file
mapping the module's resource paths to them, which qmlcachegen resolves the module through. The copies are of the sources
as they are now, so the QML can be linted again without a rebuild. qmllint matches a file to its qmldir entry (Ratings is a
singleton only through it) by the path it was given, so the copies are linted.

Run with the same command on every OS (`just lint-qml`). The qmllint and qmlcachegen calls, and the warnings, are printed as
crates/desktop/qml paths. The Qt kit is PG_QT_DIR (from the environment or .env); its tools are read from its bin folder only,
never from PATH, so another Qt on PATH cannot lint against the wrong types.
"""

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from pgtools import QT_DIR_HOW, load_env_file, not_set

REPO = Path(__file__).resolve().parent.parent
QML_SOURCES = REPO / "crates" / "desktop" / "qml"
SINGLETON_NOT_DECLARED = re.compile(r"^Type (\w+) not declared as singleton in qmldir but using pragma Singleton$")
MEMBER_NOT_FOUND = re.compile(r'^Member "(\w+)" not found on type "(\w+)"$')
WARNING = re.compile(r"^(?:Warning|Error): (.+?):(\d+):(\d+): (.*)$")


def qt_tool(qt: Path, name: str) -> str:
    """A Qt tool from the kit, never from PATH: its bin folder, then libexec (Linux keeps qmlcachegen there). Windows names
    it with .exe."""
    exe = name + (".exe" if os.name == "nt" else "")
    for folder in ("bin", "libexec"):
        tool = qt / folder / exe
        if tool.is_file():
            return str(tool)
    sys.exit(
        f"lint_qml: {exe} is not in {qt / 'bin'} or {qt / 'libexec'}: "
        "PG_QT_DIR must be the Qt kit folder (the one with bin and lib)"
    )


def newest_qmltypes() -> Path:
    """The build script's output: target/*/build/pg-desktop-<hash>/ with stable cargo,
    target/*/build/pg-desktop/<hash>/ with the pinned nightly."""
    patterns = [
        "target/*/build/pg-desktop-*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes",
        "target/*/build/pg-desktop/*/out/qt-build-utils/qml_modules/pg/gallery/plugin.qmltypes",
    ]
    found = [p for pattern in patterns for p in REPO.glob(pattern)]
    if not found:
        sys.exit("lint_qml: no pg.gallery module in target/: build the app first (just build).")
    return max(found, key=lambda p: p.stat().st_mtime)


def main() -> int:
    load_env_file()
    qt_dir = os.environ.get("PG_QT_DIR")
    if not qt_dir:
        not_set("PG_QT_DIR", QT_DIR_HOW)
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
        sys.exit(f"lint_qml: qmllint printed no JSON report (exit {report.returncode}):\n{report.stderr}")
    # The plain QML singletons (Theme, Ratings, Wheel: `singleton` lines in the qmldir that build.rs writes) and the
    # properties each one's file declares.
    singletons = {}
    for line in (module / "qmldir").read_text(encoding="utf-8").splitlines():
        if line.startswith("singleton "):
            _, name, _, path = line.split()
            text = (module / path).read_text(encoding="utf-8")
            singletons[name] = set(re.findall(r"^\s*(?:readonly\s+)?(?:default\s+)?property\s+\S+\s+(\w+)", text, re.MULTILINE))
    for file in data.get("files", []):
        for w in file.get("warnings", []):
            # cxx-qt's plugin.qmltypes describes only the Rust-backed types, so qmllint cannot see the plain QML singletons. It
            # then warns that a singleton is not declared as one, or that its properties are not found, though the qmldir
            # declares the singleton and its file declares the property. Only those two warnings, for those cases, are skipped.
            declared = SINGLETON_NOT_DECLARED.match(w["message"])
            if declared and declared.group(1) in singletons:
                continue
            member = MEMBER_NOT_FOUND.match(w["message"])
            if member and member.group(1) in singletons.get(member.group(2), ()):
                continue
            # Some findings (the import checks) come without a position: they are reported at line 0.
            add_warning(file["filename"], str(w.get("line", 0)), str(w.get("column", 0)), f"{w['message']} [{w['id']}]")

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
            sys.exit(f"lint_qml: qmlcachegen failed on {name}:\n" + "\n".join(log))
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
