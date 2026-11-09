#!/usr/bin/env python3
"""The QML tests (S-14.3, `just qmltest`): Qt Quick Test, offscreen, on the QML that stays in QML.

The components are staged as the pg.gallery module (target/qmltest/pg/gallery), the way the app has them: the stubs of the
Rust types (crates/desktop/tests/qml/stubs), and every real QML file, each with a qmldir entry, so the singleton (Ratings)
resolves as it does in the app. Without the module, a component beside a singleton's file (GridCell beside Ratings) takes
the file's own type, and Ratings.symbol is not a function. The tests import pg.gallery and check the components only; the
stubs stand in for GalleryModel, Shell and the rest.

The tests run in a host built from crates/desktop/tests/qml_host (Qt Quick Test's main, as qmltestrunner has it) with the
app's image providers as stubs, so the image:// URLs the tests pass resolve as they do in the app.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

from pgtools import REPO, WINDOWS, load_env_file, run

QML = REPO / "crates" / "desktop" / "qml"
STUBS = REPO / "crates" / "desktop" / "tests" / "qml" / "stubs" / "pg" / "gallery"
TESTS = REPO / "crates" / "desktop" / "tests" / "qml"
HOST_SOURCE = REPO / "crates" / "desktop" / "tests" / "qml_host"
STAGED = REPO / "target" / "qmltest"
HOST_BUILD = REPO / "target" / "qml_host"
OUT = REPO / "target" / "qmltest.txt"


def qt_kit() -> Path:
    """The Qt kit: QT_DIR (its bin and lib folders), from the environment or .env. Exits when it is not one."""
    qt = os.environ.get("QT_DIR", "")
    if not qt:
        sys.exit("QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in .env.")
    qmake = Path(qt) / "bin" / ("qmake.exe" if WINDOWS else "qmake")
    if not qmake.is_file():
        sys.exit(f"QT_DIR ({qt}) has no bin/{qmake.name}: point it at the Qt kit folder.")
    return Path(qt)


def stage() -> None:
    """Stages the module: the stubs, then every QML file of the app, with its qmldir entry (the singleton's is a `singleton`)."""
    if STAGED.exists():
        shutil.rmtree(STAGED)
    module = STAGED / "pg" / "gallery"
    shutil.copytree(STUBS, module)
    entries = (module / "qmldir").read_text(encoding="utf-8").splitlines()
    for file in sorted(QML.glob("*.qml")):
        shutil.copy(file, module)
        if file.stem == "Ratings":
            entries.append("singleton Ratings 1.0 Ratings.qml")
        else:
            entries.append(f"{file.stem} 1.0 {file.name}")
    (module / "qmldir").write_text("\n".join(entries) + "\n", encoding="utf-8")


def test_env(qt: Path) -> dict[str, str]:
    """The environment the host runs in: Qt's libraries first, offscreen, and on Windows the fonts' folder.

    Offscreen draws text with FreeType, which reads the fonts from QT_QPA_FONTDIR or else from <Qt>/lib/fonts. Qt no longer
    ships fonts, so that folder is missing and every text draw warns. The app sets the same folder (crates/desktop/cpp/
    gui_app.cpp); this runner doesn't go through that code, so it sets it. Linux finds fonts through fontconfig.
    """
    env = dict(os.environ)
    env["QT_QPA_PLATFORM"] = "offscreen"
    if WINDOWS:
        env["PATH"] = f"{qt / 'bin'};{env.get('PATH', '')}"
        if not env.get("QT_QPA_FONTDIR"):
            env["QT_QPA_FONTDIR"] = str(Path(env.get("WINDIR", "C:\\Windows")) / "Fonts")
    else:
        env["LD_LIBRARY_PATH"] = os.pathsep.join(p for p in [str(qt / "lib"), env.get("LD_LIBRARY_PATH", "")] if p)
    return env


def build_host(env: dict[str, str], qt: Path) -> Path:
    """Configures and builds the test host (CMake, against the Qt kit's packages). Returns its executable."""
    cmake = shutil.which("cmake")
    if cmake is None:
        sys.exit("cmake is not on PATH: install CMake (the QML test host is built with it)")
    run([cmake, "-S", str(HOST_SOURCE), "-B", str(HOST_BUILD), f"-DCMAKE_PREFIX_PATH={qt}"], env)
    run([cmake, "--build", str(HOST_BUILD), "--config", "Release"], env)
    return HOST_BUILD / ("Release" if WINDOWS else "") / ("qml_host.exe" if WINDOWS else "qml_host")


def main() -> None:
    load_env_file()
    qt = qt_kit()
    stage()
    env = test_env(qt)
    host = build_host(env, qt)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [str(host), "-input", str(TESTS), "-import", str(STAGED), "-o", f"{OUT},txt"], cwd=REPO, env=env, check=False
    )
    if OUT.is_file():
        print(OUT.read_text(encoding="utf-8"), end="")
    sys.exit(result.returncode)


if __name__ == "__main__":
    main()
