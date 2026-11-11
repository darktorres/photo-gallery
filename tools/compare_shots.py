#!/usr/bin/env python3
"""Screenshot comparison (S-14.1, `just shots`): the working tree's gallery against a reference recorded from a base commit,
over the test-hook matrix, on one fixture, pixel by pixel. Exit code 1 when anything differs unexpectedly (or a run fails).
It was compare-shots.ps1.

    python tools/compare_shots.py --record <commit>     # build <commit> once and record its screenshots as a reference
    python tools/compare_shots.py                       # working tree against the reference (the only one, or --base <commit>)
    python tools/compare_shots.py --expect apply-view   # these scenarios are declared to differ: reported, not failures
    python tools/compare_shots.py --self                # the working tree against itself: the matrix's own determinism
    python tools/compare_shots.py --only view0,apply-view

How it works:
- A reference is tools/shots/reference/<sha10>/: one PNG per scenario and manifest.json, which holds the full commit, the
  fixtures' hashes (`fixtures`: one per template, and shot.log's), the Qt folder, the wait, and each scenario's crop
  hashes and settle line. It is recorded once, from a scratch worktree at target/shots/base (built into
  target/shots/base-target). A comparison builds only the working tree. Recording over an existing reference replaces it.
- The fixture (tools/shots/fixture.py): a synthetic library (target/shots/library, made once; delete it to make another)
  and, per side, catalogs built by that side's own binary (scan, import, thumbnails at 120 and 220 px) and saved as
  templates under target/shots/sides/<side>/<template>. `default` is the library's; `broken` adds the committed files
  of tools/shots/broken (a JPEG the import fails), copied to the fixed target/shots/broken, as a second library folder,
  for the inbox scenarios. A comparison refuses a reference whose hash for a scenario's template (or for shot.log, which
  the log scenario reads) differs from this tree's. A manifest recorded before the templates has one `fixture` hash,
  the default template's, and is still compared.
- Each run starts from a copy of its side's template catalog, fresh preferences, notes.txt and tools/shots/shot.log, and
  the library as it was made: the turn and rating keys write into the original files, so restore_library puts them back
  in place (same file, so the catalog's identity holds; the timestamp is put back too), target/shots/broken included.
- A difference is written as a diff image (the reference's pixels, with the differing ones red) under target/shots/diff.
  Each run's log and output stay under target/shots/runs/<side>/<scenario>/.

The matrix is the hooks of crates/desktop/qml/TestHooks.qml, each alone, plus a few together. A run's screenshot is taken
when the window has settled (cpp/screenshot.cpp: ready, no image loading, no one-shot QML timer pending, no frame drawn
for 300 ms). --wait is the most to wait for that; a run that reaches it is reported UNSETTLED and not compared. Each run's
settle log is <png>.settle.txt, with what the check saw.

The script re-runs itself under target/shots/venv (Pillow and numpy, from tools/shots/requirements.txt), so the global
Python is not changed. The Python that makes the venv is --python, else PG_PYTHON from the environment (or .env), else
python on PATH. The Qt kit is QT_DIR (from the environment or .env); the FFmpeg folder is FFMPEG_DIR, else .deps/ffmpeg.
"""

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from pgtools import REPO, WINDOWS, load_env_file, run

HERE = Path(__file__).resolve().parent
SHOTS = REPO / "target" / "shots"
LIBRARY = SHOTS / "library"
PRISTINE = SHOTS / "library-pristine"
VENV = SHOTS / "venv"
VENV_PYTHON = VENV / ("Scripts" if WINDOWS else "bin") / ("python.exe" if WINDOWS else "python")
FIXTURE_PY = HERE / "shots" / "fixture.py"
REQUIREMENTS = HERE / "shots" / "requirements.txt"
REF_ROOT = HERE / "shots" / "reference"
# The broken template's library folder: the committed files under tools/shots/broken, copied to a fixed place (not the
# repo's, which is a worktree's when recording), since the expanded inbox shows the file's path.
BROKEN_SOURCE = HERE / "shots" / "broken"
BROKEN = SHOTS / "broken"
BROKEN_MTIME_NS = 1_717_200_000 * 1_000_000_000  # 2024-06-01 UTC: every copy has the same timestamp, whoever checked it out
# When the broken template's failures were found, as the inbox shows it: fixed, so the inbox shots don't change with the
# day the template was made (2026-01-01 09:00 UTC).
BROKEN_FOUND_AT = 1_767_258_000
# The log view's input (the `log` scenario), copied into each run's folder.
SHOT_LOG = HERE / "shots" / "shot.log"
MASCOT = (
    1  # every run shows one fixed mascot, not the random one a window picks, so the screenshots of a commit match its reference
)

# The templates: `default` is the library's catalog; `broken` is that catalog with tools/shots/broken added as a second
# library folder, scanned and imported (its file fails the import), for the inbox scenarios.
DEFAULT = "default"
BROKEN_TEMPLATE = "broken"
TEMPLATES = (DEFAULT, BROKEN_TEMPLATE)
# The manifest's key for shot.log, beside the templates': the scenarios that read it are checked against it too.
LOG_INPUT = "shot.log"


@dataclass(frozen=True)
class Scenario:
    """One entry of the matrix: the hook arguments, and the template catalog it starts from."""

    hooks: tuple[str, ...]
    template: str = DEFAULT

    def inputs(self) -> list[str]:
        """The manifest's fixture keys this scenario depends on: its template's, and shot.log's when a hook reads it."""
        return [self.template, *([LOG_INPUT] if any(LOG_INPUT in hook for hook in self.hooks) else [])]


def scenario(*hooks: str, template: str = DEFAULT) -> Scenario:
    return Scenario(hooks, template)


# The matrix: name -> the hook arguments (and the template, when it is not the default). {FIX} and {RUN} are the run's
# folder. The reference is recorded again whenever these change.
SCENARIOS = {
    "default": scenario(),
    "size120": scenario("--size=120"),
    "select5": scenario("--select=5"),
    "view0": scenario("--view=0"),
    "viewer-zoom": scenario("--view=0", "--zoom=2"),
    "viewer-loupe-borders": scenario("--view=0", "--viewer-keys=LB"),
    "viewer-zen": scenario("--view=0", "--viewer-keys=H"),
    "viewer-one-to-one": scenario("--view=0", "--viewer-keys=1"),
    "viewer-turn": scenario("--view=0", "--viewer-keys=>"),
    "viewer-rate": scenario("--view=0", "--viewer-keys=r3"),
    "filters": scenario("--filters=1"),
    "query": scenario('--query={"exts":["png"]}'),
    "sort": scenario('--sort={"key":"name","descending":true}'),
    "menu-views": scenario("--menu=views"),
    "library": scenario("--library=1"),
    "apply-view": scenario("--apply-view=alpha"),
    "save-view": scenario("--save-view=1"),
    "borders": scenario("--borders=1"),
    "find-similar": scenario("--find-similar={FIX}\\notes.txt"),
    "save-crops": scenario("--save-crops={RUN}\\crops"),
    "filters-view0": scenario("--filters=1", "--view=0"),
    "inbox": scenario("--inbox=1", template=BROKEN_TEMPLATE),
    "inbox-expanded": scenario("--inbox=1", "--inbox-expand=import", template=BROKEN_TEMPLATE),
    "dev-mode": scenario("--dev=1"),
    "log": scenario("--dev=1", "--log=1", "--log-view={FIX}\\shot.log"),
}
# The pseudo-locale (S-19.3): every line of the front end in the pseudo-locale. No reference has it, so it is run on the
# working tree only and its screenshot kept under target/shots/pseudo, for a look; it is not compared.
PSEUDO = {
    "pseudo-locale": scenario("--locale=pseudo"),
    "pseudo-library": scenario("--locale=pseudo", "--library=1"),
}


@dataclass
class Shot:
    """One scenario's result on one side: the screenshot (None when there is none), the crop hashes, and the settle line."""

    scenario: str
    png: Path | None
    crops: str
    note: str
    settle: str

    def timed_out(self) -> bool:
        return self.settle.startswith("timed out")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def git_commit(commit: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(REPO), "rev-parse", "--verify", f"{commit}^{{commit}}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        sys.exit(f"no commit {commit}")
    return result.stdout.strip()


def gallery_exe(target: Path) -> Path:
    return target / "debug" / ("gallery.exe" if WINDOWS else "gallery")


def build_gallery(tree: Path, target: Path | None) -> Path:
    command = ["cargo", "build", "-p", "pg-gallery"]
    if target is not None:
        command += ["--target-dir", str(target)]
    run(command, cwd=tree)
    return gallery_exe(target or tree / "target")


def update_base_worktree(commit: str) -> tuple[Path, str]:
    """The scratch worktree at target/shots/base, checked out at `commit`. Returns its folder and the full commit."""
    sha = git_commit(commit)
    wt = SHOTS / "base"
    subprocess.run(["git", "-C", str(REPO), "worktree", "prune"], capture_output=True, check=False)
    if not (wt / ".git").exists():
        if wt.exists():
            shutil.rmtree(wt)
        run(["git", "-C", str(REPO), "worktree", "add", "--detach", str(wt), sha])
    else:
        run(["git", "-C", str(wt), "checkout", "--detach", "--force", sha])
        run(["git", "-C", str(wt), "clean", "-fdq"])
    top = subprocess.run(["git", "-C", str(wt), "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=False)
    if top.returncode != 0 or Path(top.stdout.strip()).resolve() != wt.resolve():
        sys.exit(f"{wt} is not its own git worktree")
    return wt, sha


def restore_library() -> None:
    """Puts the library back as the fixture made it: the writeback pass changes the originals in place. Without the pristine
    copy there is nothing to restore from, and the extra-file pass below would delete the whole library: refuse."""
    if not PRISTINE.exists():
        sys.exit(f"{PRISTINE} is missing; delete {LIBRARY} and run again")
    for file in sorted(path for path in PRISTINE.rglob("*") if path.is_file()):
        live = LIBRARY / file.relative_to(PRISTINE)
        if not live.exists():
            sys.exit(f"library file {live} is gone; delete {LIBRARY} and run again")
        same = live.stat().st_size == file.stat().st_size and sha256(live) == sha256(file)
        if not same:
            live.write_bytes(file.read_bytes())
        if live.stat().st_mtime_ns != file.stat().st_mtime_ns:
            os.utime(live, ns=(live.stat().st_atime_ns, file.stat().st_mtime_ns))
    # Files the app added beside the originals (the temp files of the writes).
    for extra in sorted(path for path in LIBRARY.rglob("*") if path.is_file()):
        if not (PRISTINE / extra.relative_to(LIBRARY)).exists():
            extra.unlink()
    if BROKEN.exists():
        place_broken()


def broken_files() -> list[Path]:
    return sorted((path for path in BROKEN_SOURCE.rglob("*") if path.is_file()), key=lambda path: str(path).lower())


def place_broken() -> None:
    """target/shots/broken as tools/shots/broken has it, with the fixed timestamp. A file that is there already is written
    in place (the same file, so the catalog's identity for it holds, as restore_library keeps the library's); files the
    app added beside them are deleted."""
    for file in broken_files():
        live = BROKEN / file.relative_to(BROKEN_SOURCE)
        live.parent.mkdir(parents=True, exist_ok=True)
        data = file.read_bytes()
        if not live.exists() or live.read_bytes() != data:
            live.write_bytes(data)
        if live.stat().st_mtime_ns != BROKEN_MTIME_NS:
            os.utime(live, ns=(live.stat().st_atime_ns, BROKEN_MTIME_NS))
    for extra in sorted(path for path in BROKEN.rglob("*") if path.is_file()):
        if not (BROKEN_SOURCE / extra.relative_to(BROKEN)).exists():
            extra.unlink()


def fixture_hash() -> str:
    """The default template's identity: the pristine library's files and the generator. The lines are the same as the
    PowerShell version's (a leading separator, the file's hash), so the references recorded with it still match."""
    files = sorted((path for path in PRISTINE.rglob("*") if path.is_file()), key=lambda path: str(path).lower())
    lines = [f"{os.sep}{file.relative_to(PRISTINE)} {sha256(file)}" for file in files]
    lines.append(f"fixture.py {sha256(FIXTURE_PY)}")
    return hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest().upper()


def fixture_hashes() -> dict[str, str]:
    """The manifest's `fixtures`: each template's identity, and shot.log's. The broken template is the default one with
    tools/shots/broken added, so its hash covers the default's and the broken files' bytes."""
    default = fixture_hash()
    lines = [f"default {default}", *(f"{file.relative_to(BROKEN_SOURCE).as_posix()} {sha256(file)}" for file in broken_files())]
    broken = hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest().upper()
    return {DEFAULT: default, BROKEN_TEMPLATE: broken, LOG_INPUT: sha256(SHOT_LOG)}


def manifest_fixtures(manifest: dict) -> dict[str, str]:
    """A manifest's fixture hashes. One recorded before the templates has a single `fixture`: the default template's."""
    if "fixtures" in manifest:
        return dict(manifest["fixtures"])
    return {DEFAULT: manifest["fixture"]}


def fixture_step(*args: str) -> None:
    result = subprocess.run([str(VENV_PYTHON), str(FIXTURE_PY), *args], check=False)
    if result.returncode != 0:
        sys.exit(f"fixture.py {args[0]} failed ({result.returncode})")


def ensure_library() -> None:
    """The library, made once, and its pristine copy for restore_library."""
    if not LIBRARY.exists():
        fixture_step("library", str(LIBRARY))
    if not PRISTINE.exists():
        shutil.copytree(LIBRARY, PRISTINE)


def pin_failure_times(catalog: Path) -> None:
    """The failures the broken template recorded (its analyses, the ledger's rows, the inbox's notices), dated
    BROKEN_FOUND_AT: what the inbox shows of them is the same whenever the template is made. Only times: no row is added
    or removed, so the catalog stays as the passes left it."""
    conn = sqlite3.connect(catalog)
    try:
        with conn:
            conn.execute("UPDATE analysis SET analyzed = ?1 WHERE error IS NOT NULL", (BROKEN_FOUND_AT,))
            conn.execute("UPDATE file_failures SET first_seen = ?1", (BROKEN_FOUND_AT,))
            conn.execute("UPDATE notices SET created = ?1, updated = ?1", (BROKEN_FOUND_AT,))
    finally:
        conn.close()


def new_template(exe: Path, folder: Path, template: str) -> Path:
    """One side's template catalog, made by the side's own binary, with everything the app would do in the background done:
    a run starts with nothing left to process. The borders pass runs last, since the app runs it after the import, and it
    races the screenshot's wait while any file is still due.

    `default` is made from nothing over the library. `broken` is made incrementally from the side's default template (the
    sibling folder `default`, made first): its catalog copied, target/shots/broken added as a library folder, then the
    same passes, which do only the new folder's work. Its import failure is recorded here, in another process, so a run
    starts with it in the inbox's baseline and toasts nothing."""
    if folder.exists():
        shutil.rmtree(folder)
    catalog = folder / "catalog"
    restore_library()
    if template == DEFAULT:
        catalog.mkdir(parents=True)
        root = LIBRARY
    elif template == BROKEN_TEMPLATE:
        base = folder.parent / DEFAULT
        if not (base / "catalog").is_dir():
            sys.exit(f"the broken template is made from the default one, and {base} has none")
        folder.mkdir(parents=True)
        shutil.copytree(base / "catalog", catalog)
        place_broken()
        root = BROKEN
    else:
        sys.exit(f"no template {template}")
    steps = [["scan", str(root)], ["import"], ["thumbs", "--height", "120"], ["thumbs", "--height", "220"], ["borders"]]
    for step in steps:
        result = subprocess.run([str(exe), "--catalog-dir", str(catalog), *step], check=False)
        if result.returncode != 0:
            sys.exit(f"gallery {' '.join(step)} failed ({result.returncode})")
    if template == DEFAULT:
        fixture_step("views", str(catalog / "catalog.sqlite"))
    else:
        pin_failure_times(catalog / "catalog.sqlite")
    # The template catalog has background work left when this fails: see the fixture's report above.
    fixture_step("settled", str(catalog / "catalog.sqlite"))
    (folder / "notes.txt").write_text(
        "Not a still image: the Yandex search refuses it before anything is sent.\n", encoding="utf-8"
    )
    return folder


def hook_arguments(hooks: tuple[str, ...], run_folder: Path) -> list[str]:
    return [hook.replace("{FIX}", str(run_folder)).replace("{RUN}", str(run_folder)).replace("\\", os.sep) for hook in hooks]


def invoke_shot(exe: Path, side: str, name: str, hooks: tuple[str, ...], template: Path, wait: float) -> Shot:
    run_folder = SHOTS / "runs" / side / name
    if run_folder.exists():
        shutil.rmtree(run_folder)
    run_folder.mkdir(parents=True)
    shutil.copytree(template / "catalog", run_folder / "catalog")
    shutil.copy(template / "notes.txt", run_folder)
    shutil.copy(SHOT_LOG, run_folder)
    (run_folder / "crops").mkdir()
    restore_library()
    png = run_folder / "shot.png"
    argv = [str(exe), f"--screenshot={png}", f"--wait={wait:g}", f"--mascot={MASCOT}", *hook_arguments(hooks, run_folder)]
    env = {**os.environ, "PG_CATALOG_DIR": str(run_folder / "catalog"), "PG_PREFS": str(run_folder / "prefs.json")}
    process = subprocess.Popen(argv, env=env)
    try:
        code = process.wait(timeout=wait + 110)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        return Shot(name, None, "TIMEOUT", "timed out", "")
    crops = "\n".join(
        f"{path.name} {sha256(path)}"
        for path in sorted(run_folder.joinpath("crops").iterdir(), key=lambda p: p.name)
        if path.is_file()
    )
    # The app's own account of the save: "settled after ...", or "timed out ..." with what kept the window busy.
    settle_file = Path(f"{png}.settle.txt")
    settle = " ".join(settle_file.read_text(encoding="utf-8", errors="replace").splitlines()) if settle_file.exists() else ""
    if not png.exists():
        return Shot(name, None, crops, f"exit {code}, no screenshot", settle)
    return Shot(name, png, crops, "", settle)


def run_side(exe: Path, side: str, scenarios: dict[str, Scenario], wait: float) -> list[Shot]:
    """One side of the matrix: its template catalogs from its own binary (SHOTS/sides/<side>/<template>), then each
    scenario. Both templates are built, in order (broken is made from default), unless no scenario of the run uses
    broken (--only, or the pseudo-locale runs)."""
    needed = {entry.template for entry in scenarios.values()}
    templates = {}
    for template in TEMPLATES:
        if template == DEFAULT or template in needed:
            templates[template] = new_template(exe, SHOTS / "sides" / side / template, template)
    results = []
    for name, entry in scenarios.items():
        print(f"[{side}] {name}", flush=True)
        results.append(invoke_shot(exe, side, name, entry.hooks, templates[entry.template], wait))
    return results


def resolve_reference(base: str) -> Path:
    """The reference to compare with: the only one recorded, or the one for --base."""
    refs = [path for path in REF_ROOT.iterdir() if (path / "manifest.json").is_file()] if REF_ROOT.exists() else []
    if not refs:
        sys.exit("no reference recorded: run just shots --record <commit>")
    if not base:
        if len(refs) > 1:
            sys.exit(f"several references ({', '.join(path.name for path in refs)}): pass --base <commit>")
        return refs[0]
    folder = REF_ROOT / git_commit(base)[:10]
    if not (folder / "manifest.json").is_file():
        sys.exit(f"no reference for {base}: record it with just shots --record {base}")
    return folder


def save_reference(sha: str, results: list[Shot], wait: float, qmake: Path) -> Path:
    """Writes a reference from the base side's results. Every scenario must have settled and produced a screenshot."""
    for result in results:
        if result.png is None or result.timed_out():
            sys.exit(f"cannot record {result.scenario}: {result.note} {result.settle}")
    folder = REF_ROOT / sha[:10]
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    rows = []
    for result in results:
        if result.png is not None:
            shutil.copy(result.png, folder / f"{result.scenario}.png")
        rows.append({"scenario": result.scenario, "crops": result.crops, "settle": result.settle})
    manifest = {
        "commit": sha,
        "recorded": time.strftime("%Y-%m-%d"),
        "fixtures": fixture_hashes(),
        "qt": str(qmake),
        "wait": float(wait),
        "scenarios": rows,
    }
    (folder / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return folder


def compare_pixels(a: Path, b: Path, diff: Path) -> tuple[int | None, str]:
    """The number of differing pixels (all four channels compared), or the two sizes when they differ. The diff image has the
    differing pixels red, and is written only when there are some."""
    from PIL import Image, ImageChops

    with Image.open(a) as image_a, Image.open(b) as image_b:
        if image_a.size != image_b.size:
            return None, f"size {image_a.width}x{image_a.height} vs {image_b.width}x{image_b.height}"
        pixels_a = image_a.convert("RGBA")
        pixels_b = image_b.convert("RGBA")
    channels = ImageChops.difference(pixels_a, pixels_b).split()
    changed = ImageChops.lighter(ImageChops.lighter(channels[0], channels[1]), ImageChops.lighter(channels[2], channels[3]))
    count = sum(changed.histogram()[1:])
    if count:
        mask = changed.point(lambda value: 255 if value else 0)
        red = Image.new("RGBA", pixels_a.size, (255, 0, 0, 255))
        Image.composite(red, pixels_a, mask).save(diff, "PNG")
    return count, ""


def table(rows: list[tuple[str, str, str]]) -> str:
    header = ("Scenario", "Result", "Detail")
    widths = [max(len(row[i]) for row in [header, *rows]) for i in range(3)]
    lines = ["  ".join(text.ljust(widths[i]) for i, text in enumerate(row)).rstrip() for row in [header, *rows]]
    lines.insert(1, "  ".join("-" * width for width in widths))
    return "\n".join(lines)


def judge(name: str, before: Shot, after: Shot, expect: list[str], diff_folder: Path) -> tuple[str, str]:
    """The status and detail of one scenario, compared with the base's."""
    if not before.png or not after.png:
        status, detail = "MISSING", f"base: {before.note} current: {after.note}"
    elif before.timed_out() or after.timed_out():
        # Saved at the cap without settling: the picture may be mid-change, so it is not compared.
        status, detail = "UNSETTLED", f"base: {before.settle} current: {after.settle}"
    else:
        count, size = compare_pixels(before.png, after.png, diff_folder / f"{name}.png")
        crops_same = before.crops == after.crops
        if size:
            status, detail = "SIZE", size
        elif count == 0 and crops_same:
            status, detail = "same", ""
        else:
            status, detail = "DIFFERENT", f"pixels: {count}"
            if not crops_same:
                detail += "; crops differ"
    # A declared difference is reported as such; one that no longer differs means the declaration is stale.
    if name in expect:
        if status == "DIFFERENT":
            status, detail = "DECLARED", detail + " (declared with --expect)"
        elif status == "same":
            status, detail = "NOT-SEEN", "declared to differ, but identical"
    return status, detail


def changed_fixtures(manifest: dict, scenarios: dict[str, Scenario]) -> list[str]:
    """The scenarios whose inputs (template, shot.log) hash differently from the reference's record of them. An input the
    reference has no hash for is not checked: a reference recorded before it has none of the scenarios that use it, and
    those are reported as not in the reference."""
    recorded = manifest_fixtures(manifest)
    current = fixture_hashes()
    changed = []
    for name, entry in scenarios.items():
        keys = [key for key in entry.inputs() if key in recorded and recorded[key] != current[key]]
        if keys:
            changed.append(f"{name} ({', '.join(keys)})")
    return changed


def reference_results(manifest: dict, reference: Path, scenarios: dict[str, Scenario]) -> list[Shot]:
    results = []
    for name in scenarios:
        row = next((r for r in manifest["scenarios"] if r["scenario"] == name), None)
        png = reference / f"{name}.png"
        if row and png.is_file():
            results.append(Shot(name, png, row["crops"], "", row["settle"]))
        else:
            results.append(Shot(name, None, "", "not in the reference", ""))
    return results


def record(args: argparse.Namespace, qmake: Path, scenarios: dict[str, Scenario]) -> None:
    worktree, sha = update_base_worktree(args.record)
    base_exe = build_gallery(worktree, SHOTS / "base-target")
    ensure_library()
    results = run_side(base_exe, "base", scenarios, args.wait)
    folder = save_reference(sha, results, args.wait, qmake)
    print(f"recorded {len(results)} scenarios of {sha[:10]} in {folder}")


def compare(
    args: argparse.Namespace,
    qmake: Path,
    scenarios: dict[str, Scenario],
    pseudo: dict[str, Scenario],
    expect: list[str],
) -> None:
    current_exe = build_gallery(REPO, None)
    manifest = None
    reference = None
    if not args.self:
        reference = resolve_reference(args.base)
        manifest = json.loads((reference / "manifest.json").read_text(encoding="utf-8-sig"))
        if manifest["qt"] != str(qmake):
            print(f"warning: the reference was recorded with Qt at {manifest['qt']}; this tree uses {qmake}", file=sys.stderr)
    ensure_library()
    changed = changed_fixtures(manifest, scenarios) if manifest else []
    if manifest and changed:
        commit = manifest["commit"]
        sys.exit(
            f"the fixture changed since {commit[:10]} was recorded, for {'; '.join(changed)}; "
            f"re-record it with just shots --record {commit}"
        )

    # The sides to compare: the base is the reference, or the working tree itself with --self.
    if reference is None or manifest is None:
        base_label = "working tree (--self)"
        base_results = run_side(current_exe, "base", scenarios, args.wait)
    else:
        base_label = f"{manifest['commit'][:10]} (recorded {manifest['recorded']})"
        base_results = reference_results(manifest, reference, scenarios)
    current_results = run_side(current_exe, "current", scenarios, args.wait)
    pseudo_folder = SHOTS / "pseudo"
    pseudo_results = run_side(current_exe, "pseudo", pseudo, args.wait) if pseudo else []
    if pseudo_results:
        pseudo_folder.mkdir(parents=True, exist_ok=True)
        for shot in pseudo_results:
            if shot.png:
                shutil.copy(shot.png, pseudo_folder / f"{shot.scenario}.png")

    # The comparison, scenario by scenario.
    diff_folder = SHOTS / "diff"
    if diff_folder.exists():
        shutil.rmtree(diff_folder)
    diff_folder.mkdir(parents=True)
    report: list[tuple[str, str, str]] = []
    failed = 0
    for shot in pseudo_results:
        shown = shot.png is not None and not shot.timed_out()
        failed += 0 if shown else 1
        report.append((shot.scenario, "SHOWN" if shown else "FAILED", f"kept in {pseudo_folder}; not compared"))
    for name in scenarios:
        before = next(shot for shot in base_results if shot.scenario == name)
        after = next(shot for shot in current_results if shot.scenario == name)
        status, detail = judge(name, before, after, expect, diff_folder)
        if status not in ("same", "DECLARED"):
            failed += 1
        report.append((name, status, detail))

    summary = f"base: {base_label}\ncurrent: working tree\n{table(report)}\n"
    (SHOTS / "report.txt").write_text(summary, encoding="utf-8")
    print(summary)
    if failed:
        print(f"{failed} of {len(report)} scenarios differ or did not run; diffs in {diff_folder}")
        sys.exit(1)
    print(f"all {len(report)} scenarios as expected")


def scenarios_matching(names: list[str]) -> tuple[dict[str, Scenario], dict[str, Scenario]]:
    """The matrix and the pseudo-locale runs, narrowed by --only."""
    if not names:
        return dict(SCENARIOS), dict(PSEUDO)
    scenarios = {name: hooks for name, hooks in SCENARIOS.items() if name in names}
    pseudo = {name: hooks for name, hooks in PSEUDO.items() if name in names}
    if not scenarios and not pseudo:
        sys.exit(f"no scenario matches --only ({','.join(names)})")
    return scenarios, pseudo


def bootstrap(python: str) -> None:
    """Makes target/shots/venv if it is missing, installs the fixture's packages, and runs this script under it."""
    if not VENV_PYTHON.exists() and subprocess.run([python, "-m", "venv", str(VENV)], check=False).returncode != 0:
        sys.exit(f"venv failed (is {python} Python 3.12?)")
    pip = [str(VENV_PYTHON), "-m", "pip", "install", "--quiet", "-r", str(REQUIREMENTS)]
    if subprocess.run(pip, check=False).returncode != 0:
        sys.exit("pip install failed")
    child = subprocess.run(
        [str(VENV_PYTHON), str(Path(__file__).resolve()), *sys.argv[1:]],
        env={**os.environ, "PG_SHOTS_CHILD": "1"},
        check=False,
    )
    sys.exit(child.returncode)


def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--record", default="", metavar="COMMIT", help="build COMMIT and record its screenshots as the reference")
    parser.add_argument("--base", default="", metavar="COMMIT", help="the reference to compare with (when several exist)")
    parser.add_argument("--self", action="store_true", help="the working tree against itself")
    parser.add_argument("--wait", type=float, default=10.0, help="the most to wait for a settled window, in seconds")
    parser.add_argument("--only", default="", help="comma-separated scenario names to run")
    parser.add_argument("--expect", default="", help="comma-separated scenarios declared to differ: reported, not failures")
    parser.add_argument("--python", default="", help="the Python that makes the venv (default PG_PYTHON, else python)")
    args = parser.parse_args()
    if args.record and args.self:
        sys.exit("--record and --self do not go together")
    if args.record and args.only:
        sys.exit("--record and --only do not go together: a subset would replace the whole reference")

    load_env_file()
    qt = os.environ.get("QT_DIR")
    if not qt:
        sys.exit("QT_DIR is not set: point it at the Qt kit folder (the one with bin and lib), in the environment or in .env.")
    qmake = Path(qt) / "bin" / ("qmake.exe" if WINDOWS else "qmake")
    if not qmake.is_file():
        sys.exit(f"QT_DIR ({qt}) has no bin/qmake: point it at the Qt kit folder.")
    ffmpeg = Path(os.environ["FFMPEG_DIR"]) if os.environ.get("FFMPEG_DIR") else REPO / ".deps" / "ffmpeg"
    os.environ["PATH"] = os.pathsep.join([str(qmake.parent), str(ffmpeg / "bin"), os.environ.get("PATH", "")])
    os.environ["QT_QPA_PLATFORM"] = "offscreen"

    if os.environ.get("PG_SHOTS_CHILD") != "1":
        bootstrap(args.python or os.environ.get("PG_PYTHON") or "python")  # exits: the rest runs under the venv's Python

    scenarios, pseudo = scenarios_matching([name for name in args.only.split(",") if name])
    expect = [name for name in args.expect.split(",") if name]
    if args.record:
        record(args, qmake, scenarios)
    else:
        compare(args, qmake, scenarios, pseudo, expect)


if __name__ == "__main__":
    main()
